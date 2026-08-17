---
title: dashboard-ui
sources:
  - src/ui.rs
  - src/main.rs
  - src/iterm/watch.rs
  - src/status.rs
covers: >
  the dashboard Oko draws — its columns and where each value comes from, the keys it
  answers to, the one modal state it has, how a selection survives a changing row set, and
  the path by which rows track the window without polling
max_lines: 112
generated: 2026-08-16
---

# Dashboard UI

Five stacked lines (`src/ui.rs:draw`): a title, a rule, the table, a rule, a footer. The
title is `Oko — window N` from `ListSessionsResponse.Window.number`, with the row count
right-aligned on the same line; the footer is `↵ jump    ↑↓ select    r rename    q quit`,
replaced by the last error in red when one arrives and by `↵ commit    esc cancel    ^U
clear` while the rename editor is open.

## The table

One row per session in Oko's own window, ordered by tab index then position within the tab.

| Column | Value |
|---|---|
| `tab` | 1-based position of the tab in its window's `tabs[]`. Two sessions of a split tab share it. |
| `name` | `user.okoName` if set, else the last component of `path`, else `-`. Resolved in `src/iterm/watch.rs:snapshot`, never stored. |
| `process` | The literal **`claude`** for a row carrying a status; otherwise `jobName` **verbatim** — iTerm2 truncates that to 16 bytes and the column does not repair it. |
| `status` | Glyph, word and age (`src/ui.rs:render_status`): `◐ working`, `● waiting`, `○ ready`, `◌ stale`, followed by `>5m`, `>10m`, `>30m` or `>1h` once there is one. Empty for a row with no status. |
| `where` | `path`, with `$HOME` rendered as `~` (`src/ui.rs:abbreviate_home`). |

**The name is derived at snapshot time** (§2.10): an un-named row shows where it *is*, so
the label follows a `cd`, and storing that default at first sight would freeze it to whatever
directory the pane held when Oko started. `Row.stored_name` holds the variable; `Row.name`
holds the resolved label and is `None` on the watcher's own rows, as `Row.status` is.

**`Length(14)` on the status column is counted, not chosen.** `◐ working >10m` is 14 cells —
the glyphs are East-Asian-Ambiguous and score 1. At 13 ratatui truncates silently and `>10m`
renders `>10`, so `src/ui.rs`'s test draws the table into a `TestBackend` and asserts the
cell survives: a correct `Line::width` would not save a layout one cell too narrow.

**The process column is not a source of identity, and never was.** `jobName` is the
*deepest* foreground job, which for a Claude tab is some descendant — `node` on two
measured tabs, `rust-analyzer-pr` on two others. What makes a row read `claude` is the
presence of a status file for its session id (`claude-status.md`), nothing else.

A session missing `path` or `jobName` renders `-` in that cell rather than an empty or
omitted row. A row with no status renders **empty** there instead: a plain tab has no
status because nothing reports one for it, which is not the same as a value that failed to
arrive. Oko's own session is a row like any other, and so is a second pane of Oko's own tab.

## Keys

`↑`/`k` and `↓`/`j` move, `Home`/`g` and `End`/`G` jump to the ends, `Enter` activates the
selected session, `r` opens the rename editor, `q`, `Esc`, `^C` and `^D` quit
(`src/ui.rs:on_key`). Only `Press` events count, or a repeat would move the selection twice.

## The rename, and the only modal state

`r` opens an inline editor in the selected row's `name` cell, prefilled and *selected* — the
first character typed replaces the prefill. `Enter` commits, `Esc` cancels, `Backspace` and
`^U` edit. While it is open **every other binding is unreachable**
(`src/ui.rs:on_key_editing`): `q` types a `q`, `Enter` does not jump, `↑↓` do not move. Only
`^C`/`^D` still quit, kept as the one way out. The editor is keyed by session id, so a
changing row set cannot re-point a rename at a neighbour.

**Committing is a `Cmd::Rename`, not a write from `src/ui.rs`.** `src/main.rs:run` moves the
`Watcher`, and with it the only `Client`, into the socket thread, so the UI reaches iTerm2
only through the command channel; `src/iterm/watch.rs:rename` calls
`src/iterm/client.rs:set_variable` and updates the *watcher's* row — one written into the
UI's snapshot copy would show for a frame and vanish at the next emission. An empty name
commits JSON `null`, which unsets the variable and restores the derived default: the only way
back, and why `""` is not the encoding.

**The selection is a session id, not a row number** (`src/ui.rs:selected_index`). It is
resolved to an index at draw time, so a row set that changes underneath cannot re-point
`Enter` at a neighbour. When the selected session is gone the highlight falls to whatever
now occupies its old index (`src/ui.rs:apply`).

## How rows stay true

Three threads meeting on one channel of `AppEvent` (`src/main.rs:run`): the watcher owns
the socket and every conversation with iTerm2, one blocks in `event::read`, and the main
thread only draws. Quitting drops the receivers, which is what stops the other two.
Connecting happens **before** `ratatui::init()`, so an API-off failure is a readable
message rather than something that flashes between init and restore. `run` has a **second
entry point** ahead of all of it: `--follow` returns into `src/follow.rs` and none of this
runs — see `follow-stream.md`.

Nothing polls iTerm2. A variable-change notification patches one field of one row; a layout
change rebuilds the shape from the `ListSessionsResponse` it carries and fetches variables
only for sessions not already known (`src/iterm/watch.rs:rescan`). The watcher wakes every
100 ms to look at its command channel (`src/iterm/watch.rs:IDLE_TICK`) — that is how fast
`Enter` is served, not how fast the table refreshes.

The status column has no subscription to hang on, coming from a directory rather than from
iTerm2, so it rides that same tick: one `stat` of `~/.oko/status`, and a re-read only when
the mtime moved (`src/status.rs:Store::refresh`). One `stat` against a local directory is
not the polling the design ruled out, which was about iTerm2's API.

**One comparison decides every redraw** (`src/iterm/watch.rs:emit_if_changed`): the merged
view — shape, variables, name and status together — is rebuilt each tick and emitted only
when it differs from the last one sent, so an unrelated notification costs no redraw. It is
one comparison rather than one per source because the sources disagree about what "changed"
means: a variable or layout change knows it moved something, but a `working` crossing
`OKO_STALE_AFTER`, or an age crossing a bucket, knows nothing at all — no file is written and
nothing fires, only the clock moves. `Row.status` and `Row.name` are therefore filled at
snapshot time and never held on the watcher's own rows.

**An emission is not the same event as a visible change.** `Snapshot` equality compares
`Row.process`, which a row carrying a status never draws — it draws `claude`. So a `jobName`
re-sample emits a snapshot that renders identically. Measured 2026-08-16: **anything run in a
watched pane** moves that pane's deepest foreground job, and a shell loop calling `sleep`
once a second produced a *pair* of emissions every ~1.9 s — iTerm2's job poll catching
`sleep`, then `zsh` — some sixty a minute with nothing changing on screen. Under
`OKO_DEBUG_EMITS`, `emit_if_changed` appends one timestamp per emission to
`~/.oko/emits.log`; that file counts emissions, **not** differing redraws, so anything
measuring quietness with it must keep the watched panes idle to mean anything.

A socket error goes to the footer and the last rows stay on screen; the connection is not
retried, because a cookie is spent by the connection that used it.
