# Oko

*"The eye."* A dashboard tab inside iTerm2 that shows what every other tab in the window is
doing — and jumps to the one you press Enter on.

```
 Oko — window 0                                        5 rows
 ────────────────────────────────────────────────────────────
   tab  process           where
 ▸ 1    zsh               ~/dev/main/oko
   2    hx                /tmp
   3    node              ~/dev/main/spec-driven-dev
   3    zsh               ~/dev/main/spec-driven-dev
   4    oko               ~/dev/main/oko
 ────────────────────────────────────────────────────────────
 ↵ jump    ↑↓ select    q quit
```

One row per **session**, not per tab: a split tab is two rows sharing one tab number, and
Oko's own session is a row like any other. The table is live — a `cd`, a command starting,
a tab opened, closed, split or dragged all show up without restarting anything.

## Setup — once per machine

Oko talks to iTerm2's scripting API, which ships disabled:

**iTerm2 → Settings (⌘,) → General → Magic → Enable Python API**

iTerm2 asks for confirmation once. That dialog is the only prompt involved — no macOS
Automation grant is needed, because Oko runs inside iTerm2 and an app scripting itself
needs no grant. It takes effect immediately, with no restart.

If the API is off, Oko says so and points here rather than failing obscurely. The details —
where the socket is, how authorization works, how to reset a grant — are in
[`rules/iterm-api.md`](rules/iterm-api.md).

## Running it

```sh
cargo build --release
./target/release/oko
```

**Start it from a tab of the window you want watched.** Oko shows the window it is itself a
tab of; it has no cross-window view, by design. A good home for it is a dedicated tab you
leave open.

| Key | |
|---|---|
| `↑` `↓` (or `k` `j`) | move the selection |
| `Enter` | focus that session — its pane, its tab, its window |
| `q` / `Esc` | quit |

Enter changes which tab is focused and nothing else: nothing is typed into the target pane.

## What it does not do yet

The `process` column shows iTerm2's `jobName` — the *deepest* foreground job, truncated to
16 bytes, so a tab running Claude Code reads as `node` or whatever its subprocess tree
happens to bottom out in. Turning those rows into `claude` with a `working` / `waiting` /
`ready` status is Phase 3 of [`specs/tab_dashboard_spec.md`](specs/tab_dashboard_spec.md),
and it is the reason the tool exists.

Oko never spawns, kills, resizes or configures anything. It observes tabs you opened.

## Diagnostics

```sh
./target/release/probe          # identity, then the sessions of this window, headless
./target/release/probe watch    # print iTerm2 notifications as they arrive
```

`probe watch` subscribes to more than the dashboard does, so when something does not update
it tells you whether iTerm2 sent an event at all.
