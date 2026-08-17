# Oko

*"The eye."* A dashboard tab inside iTerm2 that shows what every other tab in the window is
doing — and jumps to the one you press Enter on.

```
 Oko — window 0                                                    5 rows
 ────────────────────────────────────────────────────────────────────────
   tab  name             process   status          where
 ▸ 1    api work         claude    ● waiting >10m  ~/dev/main/oko
   2    spec-driven-dev  claude    ◐ working       ~/dev/main/spec-driven-dev
   3    mdview           claude    ◌ stale >30m    ~/dev/main/mdview
   3    spec-driven-dev  zsh                       ~/dev/main/spec-driven-dev
   4    oko              oko                       ~/dev/main/oko
 ────────────────────────────────────────────────────────────────────────
 ↵ jump    ↑↓ select    r rename    q quit
```

One row per **session**, not per tab: a split tab is two rows sharing one tab number, and
Oko's own session is a row like any other. The table is live — a `cd`, a command starting,
a tab opened, closed, split or dragged all show up without restarting anything.

**Every row has a name**, because `where` tells three agents apart only when their
directories happen to differ — and running several agents in one repository is exactly when
they do not. Un-named, a row shows the last component of its directory and follows a `cd`,
because that is a description of where it is rather than a name. Press `r` to give it a real
one; that sticks through any later `cd`, and lives on the iTerm2 session, so it survives
restarting Oko and dies with the pane.

The `status` column is the reason the tool exists: with three agents running, it says which
one is blocked without your visiting all three.

| | |
|---|---|
| `◐ working` | the agent is doing something |
| `● waiting` | it is blocked on you — a permission, a question |
| `○ ready` | the turn is over; it wants a prompt |
| `◌ stale` | it *said* `working`, and has said nothing since. Oko does not know |

Each carries **how long it has said so** — nothing under five minutes, then `>5m`, `>10m`,
`>30m`, `>1h`. `● waiting >10m` is how long you have been blocking it; `○ ready >1h` is how
long you have been ignoring it. Buckets rather than a live counter, so a dashboard that sits
in a tab all day redraws a handful of times, not once a second.

`stale` is the honest answer to a case nothing reports: pressing Esc to interrupt a turn
fires no hook at all, so the last thing Oko heard was `working`. After
`OKO_STALE_AFTER` (default 10 minutes) the row stops claiming it. `waiting` and `ready`
never go stale — an agent that has been waiting twenty minutes is exactly the thing you
want to see, and `ready` is legitimately hours old.

**A tool still running is not silence.** If the hook recorded a tool that has not finished,
the row ages against `OKO_TOOL_STALE_AFTER` (default 45 minutes) instead, so a quiet
fifteen-minute build reports `◐ working >10m` rather than going stale mid-work. It is a
longer clock and not an exemption: an agent killed mid-tool still gives up eventually.

**Requirements:** macOS, [iTerm2](https://iterm2.com) 3.6 or later, and a Rust toolchain to
build. The status column additionally needs [Claude
Code](https://claude.com/claude-code) — everything else works without it.

## Setup — once per machine

Oko talks to iTerm2's scripting API, which ships disabled:

**iTerm2 → Settings (⌘,) → General → Magic → Enable Python API**

iTerm2 asks for confirmation once. That dialog is the only prompt involved — no macOS
Automation grant is needed, because Oko runs inside iTerm2 and an app scripting itself
needs no grant. It takes effect immediately, with no restart.

If the API is off, Oko says so and points here rather than failing obscurely. The details —
where the socket is, how authorization works, how to reset a grant — are in
[`rules/iterm-api.md`](rules/iterm-api.md).

## Setup — the status column

The `status` column comes from Claude Code itself, through hooks. Nothing is installed per
tab and no shell profile is touched; you register one command once:

```sh
cargo install --path .          # puts oko, oko-hook and probe on your PATH
oko-hook --print-settings       # prints the exact JSON block, with its absolute path
```

Merge that block into `~/.claude/settings.json` — it is a top-level `hooks` key, and hook
entries merge across settings levels, so it will not clobber a project-level block. **Oko
never edits that file itself.** Claude Code reads hooks at session start, so restart any
session you want in the table.

Install with `cargo install` rather than pointing the hooks at `target/release/oko-hook`: a
`cargo clean` would otherwise leave every Claude session on the machine running a hook that
no longer exists.

Every registration in that block is load-bearing, and two of the matchers are there to stop
the dashboard *lying* — `Notification` fires an `idle_prompt` a minute after every turn,
and `SessionStart` fires on auto-compaction in the middle of one. See
[`rules/claude-status.md`](rules/claude-status.md) for the whole vocabulary, and for the two
cases nothing reports at all.

The hook writes one small file per pane under `~/.oko/status/`, prints nothing, and exits 0
whatever happens — it cannot make itself visible in your session. Set `OKO_HOOK_DEBUG=1` to
send its errors to `~/.oko/hook.log`.

## Running it

```sh
cargo install --path .          # if you have not already, for the status column
oko
```

**Start it from a tab of the window you want watched.** Oko shows the window it is itself a
tab of; it has no cross-window view, by design. A good home for it is a dedicated tab you
leave open.

| Key | |
|---|---|
| `↑` `↓` (or `k` `j`) | move the selection |
| `Enter` | focus that session — its pane, its tab, its window |
| `r` | rename the selected row — `Enter` commits, `Esc` cancels, `^U` clears |
| `q` / `Esc` | quit |

Enter changes which tab is focused and nothing else: nothing is typed into the target pane.

## Reading the rows from another program

```sh
oko --follow          # newline-delimited JSON on stdout, no terminal involved
```

The first line names the build and the schema — `{"oko":"0.1.0","schema":1}` — and every line
after it is one snapshot: `window_number`, and a `rows` array carrying `session_id`, `tab`,
`name`, `path`, `status` and `age`, plus `claude: true` for a Claude row or `job` for a plain
one. Ages are the same buckets the table shows, never a second count, and a line identical to
the one before it is not written, so the stream is as quiet as the dashboard is.

**Three promises to a consumer.** It opens with the current state rather than waiting for
something to move; a `schema` you do not recognise means show nothing rather than a half-drawn
card; and closing your end of the pipe stops the writer within a few seconds, so a consumer
that wants Oko *optionally* can spawn it, read it, and drop it. Errors go to stderr and stdout
carries the protocol and nothing else. The full contract is in
[`rules/follow-stream.md`](rules/follow-stream.md).

`oko --version` prints a version and exits without touching iTerm2, so a consumer can tell a
build that speaks this from one that does not — being on `PATH` does not settle it.

## Acting on a row from another program

The stream reports; these two do what the table's `↵` and `r` do, without the table:

```sh
oko --activate <session-id>          # jump focus to that tab
oko --set-name <session-id> <name>   # name it
oko --set-name <session-id>          # clear the name, back to the derived default
```

Session ids are the ones `--follow` publishes — that is the join. Each connects, does the one
thing, and exits; a failure is a non-zero status and one line on stderr. Details in
[`rules/session-commands.md`](rules/session-commands.md).

## What it does not do

**Oko never spawns, kills, resizes or configures anything**, and there is no driving a
Claude session from here — no sending prompts, no answering permission requests. It
observes tabs you opened.

For a plain tab the `process` column shows iTerm2's `jobName` verbatim — the *deepest*
foreground job, truncated to 16 bytes, so `rust-analyzer-proc-macro-srv` reads as
`rust-analyzer-pr`. That column is a display value and never an identity test: a row says
`claude` because a status file exists for its session, not because of anything a process is
called.

**Two things nothing reports, and Oko does not pretend otherwise.** A *human* denying a
permission fires no event — the row reads `waiting` until the agent's next tool call or the
end of the turn, whichever comes first. And an Esc interrupt fires nothing at all, which is
what `◌ stale` is for.

## Diagnostics

```sh
probe                           # identity, then the sessions of this window, headless
probe watch                     # print iTerm2 notifications as they arrive
```

`probe watch` subscribes to more than the dashboard does, so when something does not update
it tells you whether iTerm2 sent an event at all.

## How this repo is organised

Oko was built spec-first, and the documents are part of the repo rather than an afterthought:

- **[`specs/`](specs/)** — *why* each decision was made, and the plan. Append-only once
  accepted, so a wrong turn stays visible with the correction beside it.
  [`specs/reviews/oko-001.md`](specs/reviews/oko-001.md) is the review record: eighteen rounds
  across six phases, every blocking finding and what it cost.
- **[`rules/`](rules/)** — *what is true right now*, tracking the code and corrected against
  it. [`iterm-api.md`](rules/iterm-api.md) is the one worth reading if you want to talk to
  iTerm2 yourself: the endpoint, the authorization dance, the join key, and the measured
  latencies. [`claude-status.md`](rules/claude-status.md) has the full hook vocabulary and
  the cases nothing reports.

`CLAUDE.md` describes the workflow; it points at a methodology that lives outside this repo,
so those paths will not resolve for you.

## License

MIT — see [`LICENSE`](LICENSE).

One exception: **[`proto/api.proto`](proto/api.proto) is vendored verbatim from
[gnachman/iTerm2](https://github.com/gnachman/iTerm2)** (commit `f4ca0004`), which is
GPL-2.0. It is included as the interface definition for iTerm2's scripting API — the wire
format a client has to speak — and pinned by commit and hash so a schema change shows up as
a diff. See [`proto/NOTICE.md`](proto/NOTICE.md) before redistributing.
