---
title: dashboard-ui
sources:
  - src/ui.rs
  - src/main.rs
  - src/iterm/watch.rs
covers: >
  the dashboard Oko draws — its columns and where each value comes from, the keys it
  answers to, how a selection survives a changing row set, and the path by which rows track
  the window without polling
max_lines: 55
generated: 2026-08-15
---

# Dashboard UI

Five stacked lines (`src/ui.rs:draw`): a title, a rule, the table, a rule, a footer. The
title is `Oko — window N` from `ListSessionsResponse.Window.number`, with the row count
right-aligned on the same line; the footer is `↵ jump    ↑↓ select    q quit`, replaced by
the last error in red when one arrives.

## The table

One row per session in Oko's own window, ordered by tab index then position within the tab.

| Column | Value |
|---|---|
| `tab` | 1-based position of the tab in its window's `tabs[]`. Two sessions of a split tab share it. |
| `process` | `jobName` **verbatim** — iTerm2 truncates it to 16 bytes and the column does not repair it. |
| `where` | `path`, with `$HOME` rendered as `~` (`src/ui.rs:abbreviate_home`). |

A session missing `path` or `jobName` renders `-` in that cell rather than an empty or
omitted row. Oko's own session is a row like any other, and so is a second pane of Oko's
own tab.

## Keys

`↑`/`k` and `↓`/`j` move, `Home`/`g` and `End`/`G` jump to the ends, `Enter` activates the
selected session, `q`, `Esc`, `^C` and `^D` quit (`src/ui.rs:on_key`). Only `Press` events
count, or a repeat would move the selection twice.

**The selection is a session id, not a row number** (`src/ui.rs:selected_index`). It is
resolved to an index at draw time, so a row set that changes underneath cannot re-point
`Enter` at a neighbour. When the selected session is gone the highlight falls to whatever
now occupies its old index (`src/ui.rs:apply`).

## How rows stay true

Three threads meeting on one channel of `AppEvent` (`src/main.rs:run`): the watcher owns
the socket and every conversation with iTerm2, one blocks in `event::read`, and the main
thread only draws. Quitting drops the receivers, which is what stops the other two.
Connecting happens **before** `ratatui::init()`, so an API-off failure is a readable
message rather than something that flashes between init and restore.

Nothing polls. A variable-change notification patches one field of one row; a layout change
rebuilds the shape from the `ListSessionsResponse` it carries and fetches variables only for
sessions not already known (`src/iterm/watch.rs:rescan`). A snapshot reaches the UI only
when the rows actually changed (`src/iterm/watch.rs:apply`), so an unrelated notification
costs no redraw. The watcher wakes every 100 ms to look at its command channel
(`src/iterm/watch.rs:IDLE_TICK`) — that is how fast `Enter` is served, not how fast the
table refreshes.

A socket error goes to the footer and the last rows stay on screen; the connection is not
retried, because a cookie is spent by the connection that used it.
