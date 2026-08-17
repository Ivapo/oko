---
title: session-commands
sources:
  - src/main.rs
  - src/iterm/watch.rs
covers: >
  the two things Oko does *to* a session — jump focus and set a name — the one place both
  are carried out, the two entry points that reach it, how the CLI spells them and what an
  absent name means, and why acting is a separate invocation from watching
max_lines: 55
generated: 2026-08-16
---

# Session commands

Oko does two things to a session rather than about it: **jump focus to one**, and **give one
a name**. Both are `Cmd` (`src/iterm/watch.rs:Cmd`), both are carried out by
`src/iterm/watch.rs:Watcher::execute`, and that is the only place either happens.

`execute` is public for a reason that is not convenience: the dashboard is not the only
caller. A rename from the CLI has to write the same variable and update the same in-memory
row as a rename from the table, and two implementations of that would drift — the row update
in particular is easy to omit and its absence looks like a flaky rename rather than a missing
write (see the note on `Watcher::rename`).

## Two entry points, one path

**The dashboard** sends `Cmd` down the command channel; `Watcher::run` drains it before each
read, so a keystroke never waits behind an idle socket. Errors become `Event::Error` and reach
the table, prefixed by `Cmd::what` — `jump` or `rename`.

**One-shot invocations** connect, run one command, and exit (`src/main.rs:run`):

| Invocation | Effect |
|---|---|
| `oko --activate <session>` | raises the window, selects the tab and the session |
| `oko --set-name <session> <name>` | sets `user.okoName` |
| `oko --set-name <session>` | **clears** it — the only way back to the derived default |

The clearing spelling is the absence of an argument rather than an empty string, matching
`Cmd::Rename(_, None)`: `""` would read back as a name and render blank. An explicitly empty
or all-whitespace name clears too — `parse_command` trims and maps empty to `None`, which is
what `src/ui.rs:on_key_editing` does with the editor's buffer, so neither door can reach a
blank name.

Session ids are the ones `--follow` publishes. That is the whole join: the stream names what
a consumer keys a card on, and these take that key.

## Why acting is its own invocation

`--follow` reports and does not act (`rules/follow-stream.md`), and that is not an oversight
to be corrected by widening the stream. The stream is one-directional by construction — one
pipe, one writer — so a consumer that wants to act spawns a second, short-lived process
rather than the stream growing a request channel and, with it, a reason for the reader to
write back.

Errors surface as a non-zero exit and one line on stderr: `oko: activate failed: BadIdentifier`
for an id that no longer exists, which is what a stale card looks like from the caller's side.
**`Cmd::what`'s `jump`/`rename` prefix is the dashboard's**, applied in `Watcher::run` — a
one-shot failure carries the wording of the operation that failed instead.
