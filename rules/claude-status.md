---
title: claude-status
sources:
  - src/status.rs
  - src/bin/oko-hook.rs
  - src/iterm/watch.rs
covers: >
  how a Claude Code tab reports what it is doing — the hook binary and the events it
  answers to, the status file's path and format, the identity join to a row, the two clocks
  a value ages on, what deletes a file, and the holes that remain
max_lines: 124
generated: 2026-08-16
---

# Claude status

**The coupling is one directory of small files, in one direction.** Claude Code decides
when a hook runs and runs it; Oko never queries Claude Code and Claude Code never knows Oko
exists.

## The hook

`oko-hook` (`src/bin/oko-hook.rs`), registered in `~/.claude/settings.json` with an
absolute path that `oko-hook --print-settings` fills in from `std::env::current_exe`. Oko
never edits that file itself. A binary, not a shell script, because parsing the event JSON
in shell needs `jq`.

`--help`/`-h` prints its two forms and exits 0, matched **ahead of the stdin read** — on
nothing but `--print-settings` it answered `--help` by blocking in `read_to_string` forever.

| Event (matcher) | Writes |
|---|---|
| `SessionStart` (`startup`, `resume`, `clear`) | `ready` |
| `UserPromptSubmit` | `working` |
| `PreToolUse` (`*`) | `working`, **and the tool's name** |
| `PostToolUse` (`*`) | `working`, tool cleared |
| `PermissionDenied` (`*`) — **auto mode** denied a call; the agent runs on | `working` |
| `Notification` (`permission_prompt`, `agent_needs_input`, `elicitation_dialog`, `elicitation_url_dialog`) | `waiting` |
| `Notification` (`elicitation_complete`, `elicitation_response`) | `working` |
| `Stop`, `StopFailure` | `ready` |
| `SessionEnd` | deletes the file |

**Every matcher is load-bearing**, and `src/bin/oko-hook.rs:action` repeats them as a
second line of defence — it is also the *only* filter for the two `Notification` rows,
which write opposite statuses and cannot be split by matcher. `Notification` fires for nine
types, one of them `idle_prompt`, about a minute after every turn: registered bare, `ready`
becomes unreachable and `waiting` stops meaning *this agent is blocked*. `SessionStart`
fires on `compact` — auto-compaction, **mid-turn** — which bare would announce a working
agent as done. `fork` is excluded as ambiguous.

Two facts govern how the block is written. A matcher of only letters, digits, `_`, `-`,
spaces, `,` and `|` is an exact string or `|`-list, **not a regex**, and each event matches
a different field (`tool_name`, `notification_type`, `source`); `UserPromptSubmit` and
`Stop` take no matcher at all. And `timeout` is 5 s everywhere except `SessionEnd`, whose
hooks share a 1.5 s budget that a longer per-hook timeout would *raise*.

**The hook is silent on both streams and exits 0 on every path**, its own errors included.
Stdout becomes Claude's context on `UserPromptSubmit` and `SessionStart`; stderr is shown
to the user on `SessionStart` and `SessionEnd`. `OKO_HOOK_DEBUG` diverts errors to
`~/.oko/hook.log`.

## The file, and the join

`~/.oko/status/<iterm-uuid>.json`, absolute because a hook's `cwd` is whichever project
that session is in (`src/status.rs:status_dir`). Written temp-file-plus-rename in the same
directory (`src/status.rs:write`) — Oko reads it concurrently, and the rename is also what
moves the directory's mtime.

    {"iterm_session_id":"F79BC…","claude_session_id":"…","status":"working","at":"…Z"}
    {…,"status":"working","at":"…Z","tool":"Bash"}      ← PreToolUse only

`tool` is **absent** rather than null when there is none, so every file Phase 3 wrote still
reads: `src/status.rs:Entry::from_json` takes it through an optional accessor, not the
required-field closure the other four use.

The join key is the UUID after the colon in `TERM_SESSION_ID`, then `ITERM_SESSION_ID` —
the same two names in the same order as `src/iterm/watch.rs:resolve_own_session`. **A pane
exporting neither writes no file at all**: Terminal.app or tmux has no iTerm2 identity, and
a file nothing can match is a file nothing can sweep. The Claude session id is not the join
— it tells one pane's successive sessions apart, which is what makes
`src/status.rs:remove_if_owned` safe on `/clear`, where `SessionEnd` and the successor's
`SessionStart` fire in an order nothing documents.

**A session is a Claude tab iff this directory holds a file for it** — an exact UUID match
against `src/iterm/watch.rs:Row.session_id`, never a process name.

## What removes a status, and what ages one out

1. `SessionEnd` deletes the file, conditionally as above.
2. `src/iterm/watch.rs:sweep_status` deletes any file whose session is in **no window of
   the whole `ListSessions` response** — not Oko's rows, which are window-scoped: sweeping
   against those would destroy Claude tabs' status in other windows, and two Okos would
   delete each other's files continuously. **Buried sessions count as alive.** It runs in
   `rescan`, the only place a closing tab produces an event, and covers a `kill -9`.
3. `working` older than its threshold renders `◌ stale`. **`waiting` and `ready` never age**
   — an agent waiting twenty minutes is the answer the product exists to give, and `ready` is
   legitimately hours old.

## The two clocks, and the ladder

Which threshold a `working` expires on is the one thing `tool` decides
(`src/status.rs:Entry::shown`). Without one, `OKO_STALE_AFTER` — 10 minutes. With one,
`OKO_TOOL_STALE_AFTER` — **45 minutes**, because a tool in flight is not silence: a quiet
fifteen-minute build reports `◐ working >10m` rather than `◌ stale`.

**`PreToolUse` sets the field and every other event clears it.** That rule is total over the
table above by construction — `src/status.rs:write` builds a whole `Entry` and renames it, so
there is no "leave this field alone" — and it stays total when a row is added. It is safe
because the field is consulted only on `working`, and the other statuses never age.

**45 is off the ladder deliberately** (OQ-6). At 30 minutes or 1 hour the bucket fires at the
same instant staleness does and that rung can never render; off it, a long build legibly
climbs `>5m` → `>10m` → `>30m` first. The ladder itself (`src/status.rs:Age`) is `None` under
five minutes, then `>5m`, `>10m`, `>30m`, `>1h`, and every status carries it — one clock with
one meaning, `now - at`. Buckets rather than seconds because a live counter would redraw the
table every second forever.

**One tool slot, and it degrades off.** Parallel calls mean the first `PostToolUse` clears
the field while the others run, and the row falls back to the 10-minute clock — Phase 3's
behaviour, not a wrong answer. Pressing Esc *during* a tool leaves it outstanding, so that
row claims `working` for up to the longer threshold; the next `UserPromptSubmit` corrects it
within seconds, which is why interrupt-then-walk-away is the accepted cost.

## Holes, stated

- **A human denying a permission fires nothing.** `PermissionDenied` is auto mode's denial;
  `PostToolUseFailure` needs a tool that ran. The bound is `PreToolUse` — documented order
  `PreToolUse` → `PermissionRequest` → `PostToolUse` — so a denial followed by another tool
  call clears within one call, and one followed by plain text clears at `Stop`. In between
  the row says `waiting` while the agent works.
- **A user interrupt fires nothing.** `Stop` does not fire on one and an API error fires
  `StopFailure` instead, so the row is left saying `working`. Staleness covers it.
- `PermissionRequest` was rejected for `waiting`: it fires whenever a call needs a decision,
  including auto-approved ones, and a row flashing `waiting` on every tool call is noise.
  `Notification`'s ~6 s delay is treated as a filter instead.
- Unsettled: whether Claude Code runs other tools in a batch while a permission dialog is
  open, which would let a `PostToolUse` overwrite a live `waiting`.
