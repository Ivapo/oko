---
title: mdview-file
sources:
  - src/iterm/mdview.rs
  - src/iterm/watch.rs
covers: >
  what an mdview tab has open and how Oko knows — the gate on `jobName`, why the launch
  argument is the open file and the one mdview change that would end that, the subscription
  and the single read, the loop that makes it a subscription, the map kept for sessions
  outside the window, the parser's five rules and its two answers, the clear on a job change,
  and what the stream carries
max_lines: 70
generated: 2026-09-21
---

# The file an mdview tab has open

The file each mdview tab has open is what tells them apart, so a row whose job is `mdview` reads
`mdview <file>` in the `process` cell, cut at 17 cells (`dashboard-ui.md`). **No screen is read.**

## The launch argument is the open file

mdview takes exactly one operand, refuses a second, exits on every `-` argument but its own
three flags, and opens nothing else — a link renders as text, `r` rereads the same path. So
iTerm2's `commandLine`, its record of the process's arguments, names the file for the process's
whole life. That is a fact about mdview: Helix moves off its launch arguments, which is why
`helix-file.md` reads a screen instead. **The day mdview can open a second file without exiting,
this is confidently wrong after the first switch**, and the fix is for mdview to announce its
file — a `user.` variable, read as `user.okoName` is — not for Oko to read its screen; mdview's
own `CLAUDE.md` says so. mdview over `ssh` has job `ssh`, and is not covered.

## Subscribed, not read at the transition

The gate is `jobName` exactly `mdview` (`src/iterm/mdview.rs:JOB_NAME`), which a full path, a
symlink and `exec -a` all read. **`jobName` does not move between two mdviews run back to back**
— `for f in a b; do mdview "$f"; done` changes `jobPid` and not `jobName` — so a read taken when
the job becomes `mdview` would show the loop's first file throughout. `commandLine` notifies
once per file, the middle ones included.

So the first time a session's job is `mdview`, `src/iterm/watch.rs:sync_files` subscribes its
`commandLine` and **then** reads it once, in that order so nothing between is lost; after that
the notifications keep it current. One round trip per pane that ever runs mdview, none per file,
and `OKO_DEBUG_READS` logs that read to `~/.oko/reads.log` beside the screen reads. The
subscription is attempted once per session and never cancelled. **A refused subscribe keeps
nothing** — the one read is not stored, and the pane reads plain `mdview` for the watcher's life.

The value is held **off the row**, in `src/iterm/watch.rs:Watcher`'s `command_lines`, since a
command line on `Row` would make every command in a subscribed pane an emission. And it is kept
for **every subscribed session, with or without a row**: `Watcher::apply` stores a `commandLine`
notification before it looks for a row, so a pane dragged out of the window and back comes back
with its current value rather than the one it left with.

`jobName` notifies ahead of `commandLine`, in the same millisecond, so a start in a pane already
subscribed can show one pass of plain `mdview` — a stream line with no `file` — before the name.

## The parser: five rules, two answers

`src/iterm/mdview.rs:open_file` answers a base name only from a command line it reads exactly:

1. It begins with the literal `mdview ` — argv[0]'s base name, then one space. A symlink or an
   `exec -a` name is its own first word, and may itself contain a space.
2. Exactly one argument follows: bare, with no whitespace, quote, `\`, `$` or backtick;
   double-quoted, with no `"`, `\`, `$` or backtick inside; or single-quoted, with no `'`
   inside. **No escape is admitted**, so in each shape the text is argv verbatim.
3. It does not begin with `-`.
4. It contains no control character — a tab is a legal file-name byte no cell can draw.
5. The answer is what follows the last `/`; an empty one is nothing.

Measured, iTerm2 3.7.2: whitespace, `'` or `*` gets double quotes; `"`, `\` or `$` gets single
quotes; none, bare. An argument needing both is written bare with backslashes —
`mdview both\'\"q.md` — which rule 2 refuses. **The answer is a name or `None`, never "leave it
as it was"**: a command line cannot be covered for a moment the way a screen can.

## What a row holds

`Row::file` is derived from the held value whenever `jobName` or `commandLine` changes, on an
`mdview` row and nothing else. **It never outlives the job that named it**: `jobName` can go
`mdview` → `hx` with no shell between, so `apply`'s `jobName` arm clears `file` when — and only
when — the value moved. `sync_files`, which `rescan` also calls, never clears an `hx` row's file
and an `mdview` row's only by deriving it, so a layout change cannot blank a Helix row.

`src/iterm/watch.rs:tracks_a_file` — `hx` or `mdview` — is the one predicate the table and
`src/follow.rs:row_json` both ask, so the stream carries `file` exactly where the cell draws one
(`follow-stream.md`). `track_files` turns this on for the dashboard and `--follow` only.
