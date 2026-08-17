---
title: follow-stream
sources:
  - src/follow.rs
  - src/main.rs
  - src/iterm/watch.rs
covers: >
  the JSON stream `oko --follow` writes — its header and schema, the shape of a line and
  where each field comes from, what it deliberately omits, the rule that keeps it quiet, the
  keepalive and the three ways the process ends
max_lines: 85
generated: 2026-08-16
---

# Follow stream

`oko --follow` writes the same rows the dashboard draws as **newline-delimited JSON on
stdout**: one header line, then one line per snapshot. `src/main.rs:run` takes the branch
before `ratatui::init()` and before connecting, and nothing in this mode touches the terminal.

It is a **separate process and a serialized interface** rather than a crate other programs
link, because the feature has to degrade to nothing: no Oko installed, no card view and no
error. The whole contract is that a binary named `oko` may be on `PATH`, and what follows —
plus `oko --version`, ahead of every other branch (`src/main.rs:run`), because presence is
not capability: a build predating this mode falls through to the dashboard, and on a pipe
that path panics inside `ratatui::init()` rather than reporting anything.

## The header

`{"oko":"<crate version>","schema":1}` (`src/follow.rs:header_line`), **once per stream, not
per line** — a stream is one process and one build, so the schema cannot change inside one. A
consumer meeting a `schema` it does not know is expected to render **nothing** and say so,
rather than draw the fields it recognises.

## A line

`{"window_number": N, "rows": [...]}` (`src/follow.rs:snapshot_line`), one object per session
in Oko's own window, in the table's order. Keys are alphabetical — `serde_json`'s map is
sorted, which is what makes the suppression rule below a string comparison.

| Field | Value |
|---|---|
| `session_id` | the iTerm2 session UUID — the join key, and what a consumer keys a card on |
| `tab` | 1-based tab position; two sessions of a split tab share it |
| `name` | `user.okoName` if set, else the last component of `path`; `null` when neither |
| `path` | the working directory, **unabbreviated** — no `~`, no truncation |
| `status` | `working`, `waiting`, `ready`, `stale`, or `null` for a session that is not a Claude tab |
| `age` | `">5m"`, `">10m"`, `">30m"`, `">1h"`, or `null` under five minutes |
| `claude` | `true`, **present only** on a row carrying a status |
| `job` | `jobName` verbatim, **present only** on a row without one. 16-byte truncation and all |

`claude` and `job` are exclusive, and that is the interface (`src/follow.rs:row_json`).

**Three things the schema does not do.** `age` is the bucket and never seconds. `status` is
the **effective** value, `stale` included, because that one is derived at read time from two
clocks and a consumer handed the written value would have to re-implement both. And `name`
and `path` are what Oko knows rather than what the table draws — the `-` placeholder, `~` and
the 16-cell truncation are the table's decorations.

**A row with a status carries no `job`.** That `jobName` is never displayed, is no identity
test, and on a Claude pane never moves: Claude Code hands a tool no foreground job, so the
value stays the agent process. Publishing it would repeat one constant rather than inform.

## Quietness is the writer's property

`src/follow.rs:Stream` keeps the last line it wrote and **drops one identical to it**. That is
not belt-and-braces over `src/iterm/watch.rs:emit_if_changed`: `Snapshot` equality compares
`Row.process`, which this schema omits for a Claude row, so a `jobName` re-sample reaches the
closure and serializes to the same line. Measured 2026-08-16: four idle sessions, 60 s, **zero**
lines.

The opening snapshot is written directly by `src/follow.rs:run`, not through the emission path:
`src/iterm/watch.rs:connect` ends by setting its own `emitted`, so the state at connect can
never publish as a *difference*, and a consumer would otherwise draw nothing until something
moved.

## The keepalive, and the three ways this ends

One thread writes a **bare newline every 5 s** (`src/follow.rs:keepalive`), so a closed pipe is
noticed within one interval rather than never — `emit` fires only on a change, which §2.11
designs to be rare. Keepalives are blank lines, so `grep -c .` counts the JSON ones and `wc -l`
counts both. It is deliberately **local to this mode**: a tick routed through
`src/iterm/watch.rs:Event` would reach `src/ui.rs:run`, whose `terminal.draw` sits outside the
action match, and redraw the dashboard ten times a second forever.

1. **The reader goes away.** Rust ignores `SIGPIPE`, so a closed pipe is a write error: the
   keepalive calls `std::process::exit`, and a failed snapshot write returns `false` from
   `emit` and stops `src/iterm/watch.rs:Watcher::run`. Either way the status is 0.
2. **The socket dies mid-stream.** `Event::Error` goes to **stderr**, status non-zero.
3. **Connecting fails** — the API off, no joining pane. Nothing has reached stdout yet.

Stdout carries the header, snapshots and keepalives, and nothing else, ever. Two threads write
it, so the handle is `Stdout` and **never** a `StdoutLock`: `writeln!` takes the internal lock
per call, and hoisting it out starves the keepalive and restores the orphan it prevents.

`--follow` sends no `Cmd`; it holds a live sender only because `Watcher::run` returns at once
on a disconnected channel. Nothing *here* lets a consumer act on a session — it reports, and
`rules/session-commands.md` is where acting on what it drew lives.
