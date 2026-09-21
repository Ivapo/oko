---
title: helix-file
sources:
  - src/iterm/helix.rs
  - src/iterm/watch.rs
  - src/iterm/client.rs
covers: >
  what a Helix tab has open and how Oko knows — the gate on `jobName`, the screen
  subscription, the quiet window and the ceiling, the left-edge anchor and its uniqueness
  rule, the three answers and what each does to a row, which two entry points read, and what
  the stream carries
max_lines: 110
generated: 2026-09-17
---

# The file a Helix tab has open

Three Helix tabs read `hx`, `hx`, `hx`. What tells them apart is the file each has open, and
**nothing structured reports it**: Helix sets no window title and has no hooks, iTerm2's
`commandLine` and `terminalWindowName` stay at the launch arguments through `:open` and the
file picker, and the process holds no descriptor for the file it is editing. So Oko reads one
line of the pane's screen — its status line — and one word off that line.

**This is the only place Oko reads a user interface**, and the mechanism is shaped so that
the failure is *absence*: a row that cannot be read reads plain `hx`, exactly as before this
existed. Nothing here is allowed to name the wrong file.

## The gate is `jobName`, and it is exactly `hx`

A session is tracked while `jobName` is the literal `hx` (`src/iterm/helix.rs:JOB_NAME`).
Measured 2026-09-17: iTerm2 sets it once, at 135 ms, and never moves it — not when
rust-analyzer starts and indexes, not on `:lsp-restart`, not with a real foreground child
(`:run-shell-command sleep 3`). **A language server sits outside the pane's foreground
process group, which is what `jobName` resolves**, so it stays `hx` while `deepestJob`
descends into the server and reads `node` or `rust-analyzer-pr`.

No other editor is covered: one that sets a title — Neovim's `title` option — would offer a
structured source, which is a different mechanism. Helix over `ssh` has job `ssh`.

## Subscribe, then wait for quiet

A tracked session is subscribed to `NOTIFY_ON_SCREEN_UPDATE`
(`src/iterm/client.rs:Client::watch_screen`, per session) and read with a `GetBufferRequest`
carrying `screen_contents_only` (`src/iterm/client.rs:Client::screen`), which returns the
rows' text and nothing else. `SESSION_NOT_FOUND` is an ordinary answer and returns no rows.
**The response's `cursor` is deliberately discarded**: it counts buffer rows rather than
screen rows, and the rule that used it was measured wrong twice over.

The notification carries a session id and nothing else — it cannot say what changed — so one
read per notification would be one round trip per keystroke. Instead a session is read
(`src/iterm/watch.rs:read_due_screens`, `src/iterm/watch.rs:Due`):

- **once its screen has been quiet for 250 ms**, which clears every measured gap inside a
  burst (a held key delivers updates 15–49 ms apart, a starting language server 11–17 ms) and
  sits above `src/iterm/watch.rs:IDLE_TICK`, the pace the watcher could honour a window at;
- **or 2 s after its first unread update**, so a screen that never goes quiet is still read.
  The window sits above the ~140 ms between two keystrokes deliberately, so sustained typing
  is served by this ceiling instead;
- **at once when its job becomes `hx`** (`Due::AtOnce`), because Helix drew its status line
  before the subscription existed and no update may ever follow. A further update leaves that
  mark alone rather than starting the window over.

Nothing is asked for on a timer. The check runs on **every** pass of
`src/iterm/watch.rs:Watcher::run`, not only the passes that carried a notification — the read
that matters is the one after the updates stop. **An idle Helix costs nothing**: no updates,
no reads. `OKO_DEBUG_READS` appends a timestamp per read, mdview's `commandLine` read included,
to `~/.oko/reads.log` (`src/iterm/watch.rs:log_read`) — no session id and no file name.

## The line is found by where it starts

`src/iterm/helix.rs:open_file` splits every row on `│` — `helix-tui`'s vertical separator,
drawn at every view's right edge — and takes a piece that **starts** with a space-padded
`NOR`, `INS` or `SEL` and carries a `line:col` later in that same piece. An unfocused view
writes blanks where its mode would be, so only the focused view's piece qualifies.

**Exactly one such piece on the screen, or the answer is nothing.** Buffer text sits behind a
gutter and never begins a piece; with gutters off a file's own text can, and then the screen
has two candidates and yields no answer rather than the first. Two other anchors were measured
and rejected: the cursor, which takes a counterfeit sitting between it and the real line — this
repository's own spec contains one — and the line's style, which under the `everblush` theme is
every cell's style.

The file name is what follows the mode (5 cells), the cell Helix reserves for the
language-server spinner, and `file-name`'s leading space — **counted, never trimmed**, or a
spinning pane reads `hx ⣾ main.rs`. It ends at the read-only indicator, the modification
indicator, or the first run of **two** spaces; one space cannot end it, because a path may
contain one.

## Three answers, and what each does to a row

| `src/iterm/helix.rs:Open` | the row's `file` |
|---|---|
| `File(base name)` | set to it |
| `NoFile` — Helix's `[scratch]` | cleared |
| `NoStatusLine` — no candidate, or several | **left as it was** |

The third is not the second, and the split is the point: an overlay covering the status line
must not clear a correct name, while `[scratch]` must. A failed read leaves the value alone
too; a refused subscribe or cancel is swallowed and never retried, since it serves one optional
cell and a dead watcher would cost every row.

`src/iterm/watch.rs:Row::file` is **held** on the watcher's rows and carried forward by
`rescan`, so a layout change does not blank every Helix row, and a job that stops being `hx`
is unsubscribed and its file cleared in the same pass.

## Two readers, and what the stream carries

`src/iterm/watch.rs:Watcher::track_files` is called from `src/main.rs:run`'s dashboard branch
and from `src/follow.rs:run`, and from **neither one-shot command**: `--activate` and
`--set-name` connect, act and exit without subscribing any screen, which would cost a round trip
per Helix pane in order to send one request.

`src/follow.rs:row_json` publishes `file` on a row whose job is `hx` (`rules/follow-stream.md`),
so a file switch is a line on the stream rather than one it suppresses; the table draws the same
value in its `process` cell, as `hx <file>` (`rules/dashboard-ui.md`). A dashboard and a
`--follow` in one window subscribe the same panes and each takes its own reads, so the round
trips double — cost, not incorrectness, each bounded by its own window and ceiling.

## What it costs

This rests on Helix's **default status line**. Move or remove its mode or position elements,
rename the modes, or turn gutters off, and a row reads `hx` and no file — absent, not wrong. A
Helix release that reshapes the line lands the same way, and `src/iterm/helix_fixtures/` —
screens captured from a real Helix 25.07.1 — is where that shows up first.
