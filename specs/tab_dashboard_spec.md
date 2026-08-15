---
id: oko-001
title: tab-dashboard
note: >
  The iTerm2 dashboard tab — live per-tab directory, process and Claude Code status for
  every tab in the window, with Enter to jump to the selected one.
status: accepted
last_updated: 2026-08-14

phases:
  - name: "Phase 1 — transport spike: reach the iTerm2 API from Rust"
    reviewed: 2026-08-14
    shipped: null
    cut: null
    by: null
  - name: "Phase 2 — live tab table with Enter-to-jump"
    reviewed: null
    shipped: null
    cut: null
    by: null
  - name: "Phase 3 — Claude Code status from hooks"
    reviewed: null
    shipped: null
    cut: null
    by: null

extends: null
supersedes: null
superseded_by: null
related: []
reference: >
  herdr, a tmux-like manager for multiple agent sessions. Out of scope from it: spawning,
  killing, resizing or otherwise managing sessions. Oko only observes tabs a human opened.
---

# Tab Dashboard

## 1. Goal

One tab in an iTerm2 window that answers, at a glance, **which of my other tabs needs me
right now** — and gets me there in one keystroke.

**The observable is the dashboard itself:** a live table showing what every other tab in
the window is doing — working directory and foreground process for a plain tab, and
`working` / `waiting for input` / `ready` for a tab running Claude Code — where pressing
Enter on a row moves iTerm2's focus to that tab. Anything that does not end up visible in
that table, or does not make jumping to a tab work, is mechanism rather than product.

The end state, sketched:

```
 Oko — window 0                                        5 rows
 ────────────────────────────────────────────────────────────
   tab  process                    where
 ▸ 1    claude  ● waiting          ~/dev/main/oko
   2    claude  ◐ working          ~/dev/main/spec-driven-dev
   3    claude  ○ ready            ~/dev/main/mdview
   4    nvim                       ~/dev/main/oko/src
   5    oko                        ~/dev/main/oko
 ────────────────────────────────────────────────────────────
 ↵ jump    ↑↓ select    q quit
```

Row 4 is a plain tab: it gets a process name and a directory and no status, because
nothing reports one for it. Row 5 is Oko itself — it is a session in the window like any
other and is not special-cased out. Rows 1–3 are Claude Code tabs, and the status column
is the reason the tool exists: a human running three agents wants to know which one is
blocked without visiting all three.

**Two things in that sketch are end-state, not intermediate.** The `claude` label on rows
1–3 comes from the status file, not from the process name — iTerm2 reports a *descendant*
of `claude` for those tabs, which on this machine is `node` (§2.2, OQ-2) — so before
Phase 3 exists those rows read whatever that descendant is. And the status column arrives
only with Phase 3.

### 1.1 Non-goals

- **Oko does not spawn, kill, resize or configure sessions.** It observes tabs that a
  human opened. This is the whole difference from herdr, and it is what keeps the design
  small enough to be worth building.
- **No cross-window view.** Rows are the sessions of the window Oko itself is in. A
  multi-window rollup is a different product and a different set of identity problems.
- **No scrollback, no screen content, no output preview.** See §2.7.
- **No control of a Claude Code session from the dashboard** — no sending prompts, no
  answering permission requests. Enter changes which tab is focused and nothing else.
- **Not a general process monitor.** The plain-tab row exists to give context to the
  Claude rows, not to compete with `htop`.

## 2. Design

### 2.1 Where the tab list comes from, and how Oko reaches it

iTerm2 exposes a local scripting API that reports windows, tabs and sessions, and reports
them **live** rather than only on request — it supports variable-change subscriptions, so
OQ-3's push branch is real and not an assumption.

Three facts about that API were checked on this machine (iTerm2 3.6.11, 2026-08-14) and
each one costs the implementer something:

1. **The API endpoint is not the socket in the iTerm2 support directory.**
   `~/Library/Application Support/iTerm2/iterm2-daemon-1.socket` is held by
   `iTermServer-3.6.11` — the session-restoration multiserver that keeps ttys alive
   across restarts — and is unrelated to scripting. The scripting API is a **separate
   WebSocket server inside iTerm2 itself**. Discovering its actual endpoint is a
   deliverable of Phase 1, not an assumption this document is entitled to make.
2. **The API is off by default and is off here.** Neither `EnableAPIServer` nor
   `NoSyncEnableAPIServer` exists in `com.googlecode.iterm2`, and iTerm2 holds no
   listening TCP socket. A human must turn it on.
3. **A client must authorize.** iTerm2 gates API clients behind a per-process
   authorization step with a cookie and a user-facing confirmation prompt. This sits
   between the implementer and every gate check in every phase, so it is scope, not
   background.

Oko connects to that API, finds the window it is running in, and builds one row per
session in it (§2.8).

**Finding its own window is itself unverified, and everything else rests on it.** The
intended mechanism is: take the UUID after the colon in `TERM_SESSION_ID` (§2.4), ask the
API which window contains that session. That requires the UUID iTerm2 exports into the
pane's environment to be *the same identifier* the API calls a session id. Plausible,
unconfirmable while the API is off, and load-bearing three times over — window scoping
here, navigation in §2.5, and Phase 3's status join in §2.4.

So **Phase 1 establishes the join key before it enumerates anything**, and carries
fallbacks rather than assuming the first one works. In order: the `TERM_SESSION_ID` UUID
against the API's session id; failing that, the pane's tty device, which the probe can
read for itself and which a session must know; failing that, the `wNtMpK` prefix, which
is a position and would work only until a tab is moved. **Whichever one joins is named in
§2.4 and in `rules/iterm-api.md`** — the key is a design output of the phase, not an
incidental.

### 2.2 What a plain tab reports

For each session the API exposes live per-session variables, two of which matter: the
session's current working directory, and the name of the job running in the foreground.
Oko reads those two and renders them.

**The foreground job is the *deepest* one, not the tab's top-level command.** iTerm2
resolves the foreground process group and reports the deepest job in it. A tab running
Claude Code therefore reports **a descendant of `claude`**, not `claude` itself, whenever
one exists: on this machine that is `node`, because the session runs an MCP server subtree
(`claude` → `npm exec` → `node`). A session with no MCP servers configured would surface
`claude`. The name is therefore a function of the user's configuration, which is exactly
why it cannot be an identity test. This is the single most consequential correction in
this section: it is why §1's sketch labels Claude rows from the status file rather than
the job name, and it is what OQ-2 is really about.

**No per-tab installation, no shell cooperation, nothing sourced in a profile** — a plain
row costs the user nothing beyond enabling the API once (§2.1). That is the contrast with
the hook machinery of §2.3, and it is why the table exists before any of it. It is *not*
a claim that Oko needs no setup at all.

### 2.3 What a Claude Code tab reports

A directory and a process name do not distinguish an agent that is thinking from one that
has been waiting twenty minutes for a permission answer. That distinction is the product,
and it has to come from Claude Code itself.

Claude Code runs a **hook** — a command of our choosing — on named events. Three carry
the whole status vocabulary:

| Event | What it means | Status it writes |
|---|---|---|
| `UserPromptSubmit` | a prompt was just submitted | `working` |
| `Notification` | Claude needs input, or permission for a tool | `waiting` |
| `Stop` | the turn finished; ready for the next prompt | `ready` |

Each firing writes one small status file. Oko reads those files and merges them into the
table. **Claude Code decides when a hook runs and runs it; Oko never queries Claude Code
and Claude Code never knows Oko exists.** The coupling is one directory of small files,
in one direction.

`~/.claude/settings.json` exists on this machine and has **no `hooks` key** today
(checked 2026-08-14), so Phase 3 adds one rather than merging into an existing
configuration.

### 2.4 Matching a status update to a row

A status file is useless unless Oko can say *which row it belongs to*. Two identifiers
meet in the hook, and only in the hook:

- **Claude Code's session id**, passed to every hook as JSON on standard input.
- **iTerm2's `TERM_SESSION_ID`**, exported into every pane's environment. Observed shape:
  `w0t2p0:F79BC39C-B1C1-47C3-9E9D-6820789978D9` — a window/tab/pane triple, a colon, then
  a session UUID.

The hook script runs *inside the pane*, so it can read both, and it writes both into the
status file. The UUID is what joins a status file to a row; the Claude session id is what
lets one pane's successive sessions be told apart. The window/tab/pane prefix is
deliberately **not** the join key — it encodes a position, and positions change when tabs
are reordered or moved between windows.

This join inherits §2.1's unverified identity claim: it works only if that UUID is the
API's session id. **Phase 1 settles it and writes the answer back into this section** —
if the UUID is not the key, the sentence above naming it as the join is what gets
corrected, and the hook writes whichever identifier does join.

### 2.5 Navigation

Each row carries the iTerm2 session id it was built from. On Enter, Oko calls the API's
activate operation for that session, which focuses the session, its tab, and raises its
window. The API's activate request does carry session selection, tab selection and
window-ordering flags, so the operation §1 depends on exists. Nothing else is sent, and
no key is injected into the target pane.

### 2.6 Why a dashboard tab, and not a side panel (decision, recorded)

A side panel means a split pane, and a split pane lives inside exactly one tab. To be
visible from every tab it would have to be repeated in every tab — a separate running
copy of Oko per tab, each connected to the API, each rendering the same table.

The dashboard tab pays a real cost for avoiding that: it is not visible at the same time
as your work. One copy of the program, in one fixed tab, reachable from anywhere, is the
trade taken.

### 2.7 Why hooks, and not reading the screen (decision, recorded)

The same API can return a pane's visible text, and a program could scan it for a spinner
or a permission prompt. That needs no cooperation from Claude Code at all, which is a
genuine advantage and the reason this is recorded rather than assumed.

It is rejected because the thing being matched is a **user interface**, which changes
without notice and without a version number. A cosmetic change to a spinner silently
turns every Claude row into a wrong answer — and a status dashboard that is confidently
wrong is worse than one that is absent, because it is trusted. Hooks deliver structured
data on events Claude Code names and documents; the failure mode is a status that stops
updating, which is visible, rather than one that lies.

### 2.8 One row per session, not per tab (decision, recorded)

A tab split into two panes is **two sessions**. Rows are sessions, because every
identifier this design turns on is a session identifier: `TERM_SESSION_ID` is per pane,
the status file's join key is per pane, and the activate operation targets a session. A
tab row would have to pick one of its panes' directories and statuses to show, and the
case where that choice is wrong — two agents running side by side in one split tab — is
precisely the case the product exists for.

The `tab` column therefore shows the **tab index**, and two sessions in a split tab share
one tab index while occupying two rows. Phase 2's gate checks this directly, because a
gate with no split in it would pass either implementation.

### 2.9 Rust and ratatui (decision, recorded)

The dashboard is a Rust binary rendering with ratatui. It is a long-lived foreground TUI
that redraws on events and does approximately no computation, so the choice is about the
author's fluency and a single self-contained binary, not about performance.

**This decision is independent of OQ-1.** Two of that question's three transports keep a
Rust/ratatui host untouched and change only how it talks to iTerm2. Only the third —
abandoning the API for `osascript` — would reshape the program, and it is the candidate
least likely to survive.

## 3. Open questions

- **OQ-1 — How does a Rust binary reach the iTerm2 API?** *(design call — Phase 1 exists
  to answer it)* The seed named "the iTerm2 Python API" and a Rust/ratatui stack in the
  same breath; those do not compose for free. The question has three parts, and the first
  is not optional: **where the API endpoint actually is** (§2.1 establishes only where it
  is *not*), **how a client enables and authorizes against it**, and **which transport
  Oko uses**. Candidates for the third part:
  1. **Speak the API's protocol directly from Rust.** No second runtime, one binary.
     Cost: implementing a client against a protocol whose stability is iTerm2's business.
  2. **A Python sidecar Oko spawns**, using the official library, emitting
     line-delimited JSON on stdout. Cost: a second runtime and a process to supervise,
     and the `iterm2` module is **not installed** in this machine's `python3` today.
  3. **Drop the API for AppleScript/`osascript`.** Cheapest to reach; almost certainly
     cannot deliver §2.2's live variables or §2.5's activate-by-session-id cleanly, and
     it is the one candidate that would also invalidate §2.9.

  **Try them in that order, and stop at the first that works.** The criterion is
  ordered, not a judgement call: a transport is acceptable only if it can (a) enumerate
  the sessions of another tab, (b) read both variables §2.2 needs, and (c) activate a
  session by id. Among those that qualify, prefer the one needing no second runtime —
  which is the order 1, 2, 3 above. Candidate 3 additionally reopens §2.9, so choosing it
  is a decision to re-argue the stack, not merely a transport call. **If none of the three
  reaches the API, stop and escalate**: the product as designed is not buildable, and that
  is a finding for a human rather than a problem for Phase 2 to inherit.

  **Named and rejected as a transport for the table:** `it2getvar`, shipped in
  `/Applications/iTerm.app/Contents/Resources/utilities` and already on `PATH`, reads
  session variables in-band via escape sequences and works with the API switched off. A
  pane can only address *itself* that way, so it cannot enumerate other tabs and is not a
  fourth candidate — but it is the cheapest possible way for Oko to learn its **own**
  session id, and Phase 1 should not rediscover it as a detour.
- **OQ-2 — How does Oko decide a row is a Claude Code tab?** *(design call — blocks Phase
  3)* Not by process name. iTerm2 reports the **deepest** foreground job (§2.2), so a
  Claude tab surfaces as `node`, and on this machine a `caffeinate -i claude` child sits
  in the same process group as its `claude` parent. Name-matching cannot be made reliable
  against a subprocess tree the agent is free to change. Proposed resolution, to be
  confirmed in Phase 3's review: **a session is a Claude tab iff a fresh status file
  exists for it**, and the process name is only ever a display value. This inverts the
  seed's design, which checked the name first.
- **OQ-3 — Does the table refresh on a timer, or does the API push changes?** *(design
  call — Phase 2)* The API supports variable-change subscriptions, so the push branch
  exists; whether it covers both variables §2.2 needs is what Phase 1's spike observes.
  If Oko polls instead, **the interval must be ≤1 s**, because Phase 2's gate is keyed to
  a 2-second wall-clock bound and a slower interval fails a gate the spec would otherwise
  never have told the implementer how to satisfy.
- **OQ-4 — What removes a status file when its session is gone?** *(design call —
  blocks Phase 3)* A closed tab leaves its last status behind. Left alone, the directory
  accretes files forever and a crashed session reads as `working` indefinitely.
  Candidates: Oko deletes files whose session id is absent from the API's session list on
  each refresh; or every status carries a timestamp and stale entries render as `unknown`
  rather than as their last value. These are not exclusive.

## 4. Implementation phases

Strictly sequential. Phase 2 cannot be planned until Phase 1 has chosen a transport;
Phase 3 merges into the table Phase 2 builds.

### Phase 1 — transport spike: reach the iTerm2 API from Rust
*Produces the observable: **no**, and this is the argument. Every later phase is keyed to
an answer that does not exist yet — §2.1 establishes only where the endpoint is not, the
API is disabled on this machine, the authorization handshake is uncharted, and §2.4's
identity claim is unverified. A phase that bundled this spike with the table would be a
phase whose later bullets could not be planned until its first bullet had been run, which
is not one plan-mode pass. Its output is a throwaway-grade probe and a decision recorded
in §3, not a user-facing artifact.*

- **Scope:**
  - `cargo init` the crate: `Cargo.toml`, `src/main.rs`, ratatui not yet a dependency.
  - Document, in `rules/iterm-api.md`, the human steps to enable the API and what the
    authorization prompt asks — a second person must be able to reproduce the setup.
  - A minimal non-TUI binary (`src/bin/probe.rs`) that connects to the API by whichever
    transport OQ-1 selects, and does exactly three things, **in this order**:
    1. **Establishes the join key.** Prints its own `TERM_SESSION_ID`, and beside it the
       identifier the API gives for the same pane, so the two can be compared by eye
       rather than by a match/no-match verdict. If the UUID is not it, works down §2.1's
       fallbacks — tty device, then the `wNtMpK` prefix — and prints which one joins.
    2. **Enumerates**, using that key to scope to Oko's own window: one line per session,
       with session id, working directory, and foreground job name.
    3. **Activates**, given a session id as an argument.
  - Record in §3: OQ-1's resolution — endpoint, enablement, authorization, chosen
    transport — the join key from step 1, and what was observed about subscribing to
    variable changes (OQ-3). Correct §2.4 if the join key is not the UUID.
  - No table, no ratatui, no selection, no key handling, no status files.
- **Exit gate:** Setup is **two** iTerm2 windows. The first holds Oko's own tab plus
  three others — an interactive `zsh` in `~/dev/main/oko`, an interactive `zsh` in
  another directory, and `nvim`. The second holds at least one tab, and exists so that
  "scoped to my own window" is distinguishable from "listed everything"; with one window
  the two are the same output. All four checks pass:
  1. **Identity.** The probe names the identifier that joins its own pane to an API
     session, and that identifier resolves to its own pane. Passing does **not** require
     the answer to be the `TERM_SESSION_ID` UUID — any of §2.1's candidates counts, and
     which one it was is the phase's deliverable. The check **fails** only if none of
     them joins, and that is an escalation to a human, not a Phase 2 problem: §2.4's
     status join and §2.5's navigation both rest on there being such a key.
  2. **Scoping.** The probe prints exactly four lines — the three tabs plus its own — and
     **no line from the second window**.
  3. **Values.** Each line's directory equals `pwd` run in that tab. Each line's
     foreground job name equals the **basename** of that tab's deepest foreground process:
     `zsh`, `zsh`, `nvim`. (Basename, because macOS `ps -o comm=` prints a full path:
     `/bin/zsh`, not `zsh`. These tabs deliberately have no child process tree, so "the
     foreground process" has one unambiguous answer; a Claude tab does not, which is
     OQ-2.) For the probe's **own** line, either `zsh` or the probe's binary name is
     accepted — iTerm2 refreshes its process cache on a poll, so a probe that connects
     and enumerates quickly can legitimately still be reported as its parent shell, and
     that is the one literal here that could fail a correct implementation.
  4. **Activation.** Passing the `nvim` line's session id back to the probe makes that
     tab the focused tab, and raises the first window if the second was focused.
- **Close-out:** seeds `rules/iterm-api.md` — endpoint, how a human enables the API,
  **how the per-process authorization works including how to revoke or reset a grant**
  (the permissions list is resettable, and a second person whose prompt does not reappear
  otherwise has an unattributable failure), the chosen transport, the join key from
  check 1, and the variables and operations Oko uses. Resolves OQ-1 in §3, records what
  OQ-3 observed, and corrects §2.4 if the join key is not the UUID. Commit plan and
  reconciliation step are stated in the phase's plan, per `CLAUDE.md`.

### Phase 2 — live tab table with Enter-to-jump
*Produces the observable: **yes** — a live, navigable dashboard for every session in the
window. It is the observable minus its status column, and it is the first phase a human
can use.*

- **Scope:**
  - A client module (`src/iterm/mod.rs` and below) wrapping Phase 1's proven transport:
    connect, resolve Oko's own window, enumerate its sessions, subscribe or poll for
    changes, activate a session by id.
  - A ratatui table over those rows: tab index, process, directory (§1's sketch, without
    the status column). Up/down selection, Enter activates, `q` quits.
  - Rows track reality: a session opened, closed, split off, or `cd`-ed into a new
    directory is reflected without restarting Oko (mechanism per OQ-3).
  - No status column, no hook machinery, no status files.
- **Exit gate:** With four tabs in one window — an interactive `zsh` in `~/dev/main/oko`,
  an interactive `zsh` elsewhere, `nvim`, and Oko itself — plus one of them split:
  1. Splitting the `nvim` tab into two panes produces **five rows**, and the two rows from
     the split tab **share one tab index** (§2.8).
  2. Each row's directory equals `pwd` in that pane; each row's process equals the
     basename of that pane's deepest foreground process, by the same rule and the same
     unambiguous choice of tabs as Phase 1's gate.
  3. `cd` in one pane, and close another tab: both rows correct **within 2 seconds**,
     measured by stopwatch from the keystroke, with Oko never restarted.
  4. Enter on the `nvim` row makes that tab the focused tab, and Enter on a row of the
     split tab focuses **that pane**, not merely the tab.
- **Close-out:** seeds `rules/dashboard-ui.md` (the table, the key bindings, the refresh
  path) and updates `rules/iterm-api.md` for anything the client learned. Resolves OQ-3
  in §3.

### Phase 3 — Claude Code status from hooks
*Produces the observable: **yes** — it completes it. The status column is the column the
project exists for; Phase 2 is the frame it hangs in.*

- **Scope:**
  - Resolve OQ-2 and OQ-4 during this phase's review round, not during implementation.
  - A hook script, committed to this repo, that reads Claude Code's JSON on stdin and
    `TERM_SESSION_ID` from the environment and writes one status file per session,
    carrying: iTerm2 session UUID, Claude session id, status, and a timestamp.
  - Registration of that script for `UserPromptSubmit`, `Notification` and `Stop` in
    `~/.claude/settings.json`, which currently has no `hooks` key — plus a documented
    way for a user to install it, since the settings file is outside this repo.
  - Oko reads the status directory and merges each status onto the row whose session id
    matches (§2.4), rendering the status column of §1's sketch — including labelling
    those rows `claude` rather than `node`, per OQ-2's resolution.
  - Staleness handling, per OQ-4's resolution.
- **Exit gate:** With **two** Claude Code sessions running in two tabs of one window:
  1. Submitting a prompt in tab A flips A's row to `working` within 2 seconds, and **B's
     row does not change** — the cross-talk check, and the one most likely to fail.
  2. A tool-permission prompt in tab A flips A's row to `waiting`.
  3. Turn completion in tab A flips A's row to `ready`.
  4. Driving B through the same three transitions moves only B's row.
  5. Both rows read `claude` in the process column, not the descendant job name iTerm2
     reports for them (§2.2 — `node` on this machine, whatever the session's MCP
     configuration produces on another).
  6. Closing tab A removes its row, and leaves no status behind that could reattach to a
     later session in the reused pane.
- **Close-out:** seeds `rules/claude-status.md` (the hook script, the status file format,
  the identity join, staleness). Updates `rules/dashboard-ui.md` for the new column, and
  the `CLAUDE.md` observable line if the status vocabulary changed. User-facing install
  instructions for the hook are part of this phase, not a follow-up.
