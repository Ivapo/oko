---
id: oko-001
title: tab-dashboard
note: >
  The iTerm2 dashboard tab — live per-tab directory, process and Claude Code status for
  every tab in the window, with Enter to jump to the selected one.
status: accepted
last_updated: 2026-09-02

phases:
  - name: "Phase 1 — transport spike: reach the iTerm2 API from Rust"
    reviewed: 2026-08-14
    shipped: 2026-08-14
    cut: null
    by: null
  - name: "Phase 2 — live tab table with Enter-to-jump"
    reviewed: 2026-08-14
    shipped: 2026-08-15
    cut: null
    by: null
  - name: "Phase 3 — Claude Code status from hooks"
    reviewed: 2026-08-15
    shipped: 2026-08-15
    cut: null
    by: null
  - name: "Phase 4 — what a row says: a name, and how long it has said it"
    reviewed: 2026-08-15
    shipped: 2026-08-16
    cut: null
    by: null
  - name: "Phase 5 — a stream another program can draw"
    reviewed: 2026-08-16
    shipped: 2026-08-16
    cut: null
    by: null
  - name: "Phase 6 — acting on a row from outside the dashboard"
    reviewed: 2026-08-17
    shipped: 2026-08-17
    cut: null
    by: null
  - name: "Phase 7 — publishing it: a crate a stranger can install, and what it says it is"
    reviewed: 2026-09-02
    shipped: 2026-09-02
    cut: null
    by: null
  - name: "Phase 8 — the binaries are the product: no library surface in the published crate"
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

**CORRECTED 2026-08-15 (Phase 3's review round; swept in at its close-out).** That
vocabulary is three values and the shipped one is four. `◌ stale` is the fourth — a
`working` that has stopped being refreshed, which is what an Esc interrupt leaves behind
because it fires no hook at all (OQ-4 (c)). The sentence above and the sketch below both
predate it, and the sketch's header line predates the status column itself, naming three
columns for a table that draws four.

The end state, sketched — **corrected to what shipped**:

```
 Oko — window 0                                                    5 rows
 ────────────────────────────────────────────────────────────────────────
   tab  name             process   status          where
 ▸ 1    api work         claude    ● waiting >10m  ~/dev/main/oko
   2    spec-driven-dev  claude    ◐ working       ~/dev/main/spec-driven-dev
   3    mdview           claude    ◌ stale >30m    ~/dev/main/mdview
   4    src              nvim                      ~/dev/main/oko/src
   5    oko              oko                       ~/dev/main/oko
 ────────────────────────────────────────────────────────────────────────
 ↵ jump    ↑↓ select    r rename    q quit
```

**CORRECTED 2026-08-16 (Phase 4's close-out).** The sketch above now carries the `name`
column and the age, both of which Phase 4 added; row 1 is named and rows 2–5 show the derived
default, the last component of `where`. Nothing else about it changed.

Row 4 is a plain tab: it gets a process name and a directory and no status, because
nothing reports one for it. Row 5 is Oko itself — it is a session in the window like any
other and is not special-cased out. Rows 1–3 are Claude Code tabs, and the status column
is the reason the tool exists: a human running three agents wants to know which one is
blocked without visiting all three. Row 3 shows the fourth value: Oko heard `working` and
has heard nothing since, and says so rather than keeping a claim it can no longer support.

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

Claude Code runs a **hook** — a command of our choosing — on named events. Ten registrations
carry the status vocabulary, and the shape of the table is *not* three events for three
statuses. **Every matcher below is load-bearing**; each was verified against Claude Code's
hooks reference on 2026-08-15, and the ones that are missing are what make a row lie.

| Event (matcher) | What it means | Writes |
|---|---|---|
| `SessionStart` (`startup`, `resume`, `clear`) | a session began or was reset | `ready` |
| `UserPromptSubmit` | a prompt was just submitted | `working` |
| `PreToolUse` (any tool) | a tool call is starting | `working` |
| `Notification` (`permission_prompt`, `agent_needs_input`, `elicitation_dialog`, `elicitation_url_dialog`) | Claude is blocked on a human | `waiting` |
| `Notification` (`elicitation_complete`, `elicitation_response`) | the human answered it | `working` |
| `PostToolUse` (any tool) | a tool ran, so a permission was granted | `working` |
| `PermissionDenied` (any tool) | **auto mode** denied a call; the agent runs on | `working` |
| `Stop` | the turn finished; ready for the next prompt | `ready` |
| `StopFailure` | the turn ended on an API error | `ready` |
| `SessionEnd` | the session is over | deletes the file |

**Most of those rows exist because the naive three-event version writes a status that
lies**, which §2.7 rejects screen-scraping to avoid — so they are corrections, not
completeness for its own sake:

- **`Notification` must carry a matcher.** It fires for nine notification types, and one of
  them is `idle_prompt` — "Claude finished responding about 60 seconds ago and you haven't
  typed since". Subscribed bare, every idle agent flips `ready` → `waiting` a minute after
  each turn, `ready` becomes unreachable in the steady state, and `waiting` stops meaning
  *this agent is blocked* — the one distinction §1 says the tool exists for. Gate check 9
  is the check that sees it; nothing shorter can.
- **`SessionStart` must carry one too**, for the same reason in the opposite direction. Its
  matchers are `startup`, `resume`, `clear`, `compact` and `fork`, and **`compact` fires on
  auto-compaction in the middle of a turn** — registered bare, every compaction of a long
  turn tells a human "this agent is done, go prompt it" while it works. `fork` is excluded
  as ambiguous rather than wrong; the next real event corrects it either way.
- **Something has to clear `waiting`.** Granting a permission fires no notification of its
  own, so without `PostToolUse` the row reads `waiting` for the rest of a turn the agent is
  actively working through.

**CORRECTED 2026-08-15 (during Phase 3's implementation): `elicitation_url_dialog` was
missing.** The reference documents nine notification types; the round-2 rebuild of this
table read five of them into the `waiting` matcher and did not carry this one, which fires
when an MCP server asks the human to open a browser URL and is `elicitation_dialog`'s exact
sibling — same blocked-on-a-human meaning, same ~6 s gate. Left out, that row reads
`working` while it waits, which is the confidently-wrong failure §2.7 rejects
screen-scraping to avoid. Added to the table above and to the matcher. This is the third
correction to arrive by the same route as round 2's two: **an event was reasoned about
without its full list of types being read.**

**Two endings fire nothing, and both are recorded as holes rather than papered over:**

- **A human denying a permission.** `PermissionDenied` is *not* that event — it fires "when
  auto mode denies a tool call" — and `PostToolUseFailure` fires only for a tool that ran
  and errored, which a denied call never does. Nothing announces a human's *no*. The
  bound is `PreToolUse`, which is in the table for this reason: it fires before the next
  tool call (the documented order is `PreToolUse` → `PermissionRequest` → `PostToolUse`),
  so a denial that Claude follows with another tool call clears within one call, and one it
  follows with plain text clears at `Stop`. In between, the row says `waiting` while the
  agent works. That window is the residual lie, and it is smaller than the turn.
- **A user interrupt (Esc).** `Stop` hooks "don't fire on user interrupts" and an API error
  fires `StopFailure` instead, so an interrupted turn is left saying `working` with nothing
  to correct it. **Nothing covers this**, and it is what OQ-4's staleness rule is for rather
  than something an eleventh registration fixes.

`PermissionRequest` was considered for `waiting` and rejected: it fires whenever a tool
call *needs a decision*, which on a permissive settings file may include calls that are
auto-approved, and a row that flashes `waiting` on every tool call is noise. `Notification`
fires when the human-facing prompt actually appears — about six seconds after Claude stops
seeing typing, which is a delay the gate has to know about (check 2) and is arguably the
right filter anyway: a permission answered in three seconds never needed a dashboard.

**The hook writes nothing to standard output.** For `UserPromptSubmit` and `SessionStart`,
Claude Code adds plain-text stdout to Claude's context — so a stray `echo` in this hook
silently prepends text to the user's prompt in every Claude session on the machine. It
writes its file, and exits 0 whatever happens.

Each firing writes one small status file. Oko reads those files and merges them into the
table. **Claude Code decides when a hook runs and runs it; Oko never queries Claude Code
and Claude Code never knows Oko exists.** The coupling is one directory of small files,
in one direction.

`~/.claude/settings.json` exists on this machine and has **no `hooks` key** today
(checked 2026-08-14, re-confirmed 2026-08-15), and this repo has no project-level
`.claude/settings.json` to collide with, so Phase 3 adds a key rather than merging into an
existing configuration.

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

**CONFIRMED 2026-08-14 (Phase 1).** It is the UUID. A session's `id` variable equals the
part of `TERM_SESSION_ID` after the colon, so nothing above needed correcting. Both
fallbacks also join and are recorded in `rules/iterm-api.md`, which matters because it
makes "none of the three joins" a real failure rather than a formality — but `id` is the
key, and Phase 3's hook writes that.

**And it is one identifier, not two.** `ListSessions`' `unique_identifier` and the
session's `id` variable carry the same string, so a row stores one id and §2.5's activate
takes the same value the status join uses. Worth stating because the two arrive by
different paths and an implementer holding both would be right to wonder. **The evidence
is narrower than the claim sounds**, and the difference matters to Phase 3 rather than to
Phase 2: the probe prints both values only for the four sessions of its own window, plus a
mismatch diagnostic on the joined session that never fired. Four sessions and one
assertion, not eleven.

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

**Where that index comes from (established 2026-08-14, during Phase 2's review round).**
**The API exposes no tab index, so it is the 1-based position of the tab in
`ListSessionsResponse.Window.tabs[]` and nothing else.** Two other sources are within
reach and both are wrong. `ListSessionsResponse.Tab` carries `tab_id`, an opaque string,
and tab scope has only `id`, `title` and the `titleOverride`/tmux fields — no index
anywhere. The sibling `Window` message *does* carry `number`, which makes the absence look
deliberate rather than an oversight; and the `tabIndex` and `tabNumber` symbols in the
iTerm2 3.6.11 binary are Objective-C selectors and ivars
(`asyncCreateTabWithProfile:…tabIndex:`, `_tabNumberForItermSessionId`), not variable
names. And the `t` component of `termid` is a **monotonically increasing id, not a
position**: Phase 1 observed `t0 t1 t2 t3 t4 t5 t8` across seven tabs of one window, so it
already disagreed with the tab bar by three before anyone tried to use it.

That leaves one unverified property, and it is the reason Phase 2's gate says what it
says: **nothing has established that `tabs[]` is in tab-bar display order** rather than
creation order or something else, and reordering tabs is the case that would tell them
apart. So the gate compares the column against iTerm2's own tab bar rather than against
itself — a check that two split rows merely *share* a number passes for all three
candidate sources, including the two ruled out here.

### 2.9 Rust and ratatui (decision, recorded)

The dashboard is a Rust binary rendering with ratatui. It is a long-lived foreground TUI
that redraws on events and does approximately no computation, so the choice is about the
author's fluency and a single self-contained binary, not about performance.

**This decision is independent of OQ-1.** Two of that question's three transports keep a
Rust/ratatui host untouched and change only how it talks to iTerm2. Only the third —
abandoning the API for `osascript` — would reshape the program, and it is the candidate
least likely to survive.

### 2.10 What a row is called (decision, recorded — added by Phase 4)

Three agents in three repositories are told apart by `where`. **Three agents in one
repository are not** — two worktrees of the same checkout, two panes of a split tab, or
simply two `claude` sessions started in the same directory — and that is precisely the case
§1 exists for: a human running several agents wants to know *which one* needs them. The
`where` column answers that only by accident of the directories happening to differ.

Nor does iTerm2's own tab bar help. A Claude Code tab's title is whatever iTerm2 derives
from the job, which §2.2 established is some unfamiliar descendant — `node`,
`rust-analyzer-pr`. The tab bar is the one place a human already looks, and for these tabs
it says nothing.

So every row carries a **name**.

**The default is derived, not stored.** An un-named row shows the last component of its
`path`, computed at render time. `cd` somewhere else and the label follows, because it is
not a name — it is a description of where the row currently is. Storing that default at
first sight would freeze it to whatever directory the pane happened to be in when Oko
started, which is the wrong answer for a shell a human navigates.

**An explicit rename is stored, and it is then the name** — through any subsequent `cd`,
because at that point a human has said what this row *is*, which no directory change
invalidates.

**Where it is stored: `user.okoName`, a variable on the iTerm2 session itself**, set through
`VariableRequest`'s `set` field, which `src/iterm/client.rs:variables` already carries and
has never used. Names set this way must begin with `user.` (`VariableResponse.INVALID_NAME`
says so), which is why the key is spelled that way.

The alternative — a file, as §2.3's statuses use — was rejected, and the reason is that
Phase 3 already paid for the lesson. A file keyed by session id needs its own sweep, its own
answer to "what if the pane is gone", and its own concurrent-write discipline; OQ-4 needed
*three* mechanisms to make that safe for statuses. A session variable needs none of them: it
dies with the pane, so nothing sweeps it; iTerm2 owns the concurrency; and **two Oko
instances see one value with no sync protocol at all**, which is what makes a name set in
one place appear in another.

**Renaming does not touch the real tab title.** `session.set_name` and `tab.set_title` both
exist in the API and both are deliberately unused here: changing what a human sees in their
own tab bar is configuring a session they opened, which §1.1 rules out, and a title set by a
program that then exits stays set. Recorded so a later pass does not quietly regress it —
mirroring the name into the tab bar is a decision to re-argue §1.1, not a convenience.

**Clearing a name is setting it to JSON `null`**, which unsets the variable and reads back
*absent* — measured, not assumed (OQ-5). The empty string is deliberately not the encoding:
`src/iterm/client.rs:decode_json_value` maps `""` to `Some("")`, which satisfies "the
variable if set" and would render a blank name that no further rename could escape.

A name dies with its pane. That is correct rather than a limitation: it names *this agent's
work*, and the next thing to occupy that pane is not that work.

### 2.11 How long it has been true (decision, recorded — added by Phase 4)

§1's motivating sentence is "an agent that has been **waiting twenty minutes**". The shipped
table shows no minutes at all. A row blocked for twenty seconds and one blocked for twenty
minutes render identically, and only one of them is a question.

So every status carries an age, and **it is one clock with one meaning**: time since the
last hook fired for that session — `now - at`, the same subtraction staleness already makes
(§2.3, OQ-4 (c)). Read across the four values it stays coherent:

| | reads as |
|---|---|
| `● waiting >10m` | how long you have been blocking it |
| `◐ working >5m` | how long since it last did anything |
| `○ ready >30m` | how long you have been ignoring it |
| `◌ stale >10m` | how long since the last credible signal — here the age *is* the reason |

**It is shown in buckets, not in seconds, and the buckets are load-bearing.** Nothing under
five minutes, then `>5m`, `>10m`, `>30m`, `>1h`. An age only appears once it starts being a
question, so a row that just changed stays clean.

**This is what keeps Oko quiet, which is a requirement and not a preference.** A live
seconds counter would redraw the table every second, forever, for a program whose whole
purpose is to sit in a tab all day. With buckets the *displayed* value changes a handful of
times a day, and `src/iterm/watch.rs:emit_if_changed` — which already rebuilds the merged
view on every tick and emits only on a difference — fires exactly at a boundary and is
silent otherwise. **No timer is added and none is wanted**: a periodic re-render would
redraw when nothing had changed, which is the same defect wearing a different hat.

One interaction to know. With `OKO_STALE_AFTER` at its 10-minute default, a `working` can
only ever reach `>5m` before becoming `stale`, so the upper rungs are exercised by
`waiting`, `ready` and `stale`. That is not a bug, and §2.12 is why it is not the whole
story either.

### 2.12 A tool in flight is not silence (decision, recorded — added by Phase 4)

OQ-4 (c) ages a `working` out because a user interrupt fires no hook, and it accepts in
writing that "**one quiet 15-minute build goes stale mid-work**". That acceptance was right
when the only evidence was a timestamp. It is no longer, because the hook is already told
`tool_name` on `PreToolUse` and `PostToolUse` and therefore knows something the timestamp
cannot express: whether a tool is **outstanding** — started, not yet finished.

So the status records the tool in flight, and **a row with one does not go stale on the
ordinary clock**. A fifteen-minute build reports `◐ working >10m`, which is true, instead of
`◌ stale`, which is false.

**The lifecycle is one sentence, and it has to be, because the writer has no third option.**
`src/status.rs:write` builds a whole `Entry` and renames it over the file, so there is no
"leave this field alone" — every event that writes a status decides the field. Rather than
enumerate ten answers and leave the eleventh event to a guess: **`PreToolUse` sets it, and
every other event clears it.** That is total over §2.3's table by construction, and it stays
total when a row is added to that table.

**The field is only ever consulted on `working`**, which is what makes the simple rule safe.
`waiting` and `ready` are already staleness-exempt (OQ-4 c), so a `Notification` clearing a
tool that is genuinely still running costs nothing — that row is not ageing anyway. The one
case the rule gives up is `elicitation_complete` → `working`, which returns to the ordinary
10-minute clock while an MCP call may still be running; that is exactly today's behaviour,
so it is a non-regression rather than a new hole.

**It is a longer clock, not an exemption.** If an agent is killed mid-tool no `PostToolUse`
ever arrives, and an unbounded exemption would leave that row claiming `working` forever —
trading §2.7's confidently-wrong failure for a slower version of itself. A second threshold,
`OKO_TOOL_STALE_AFTER` (default **45 minutes**, derived in OQ-6), bounds it.

**Parallel tool calls degrade it, safely.** One slot holds one tool, so if Claude Code runs
several at once the first `PostToolUse` clears the field while the others are still running,
and the row falls back to the 10-minute clock. That is this mechanism failing *off* — back
to the behaviour Phase 3 shipped — rather than failing into a wrong answer, which is why one
slot is enough. `rules/claude-status.md` already logs the batching question as unsettled.

**The residual, stated.** Pressing Esc *during* a tool call leaves that tool outstanding, so
such a row claims `working` for up to the longer threshold rather than the shorter one —
strictly worse than today for that case. It is accepted because of when it happens: a human
who has just pressed Esc is at the keyboard and about to type, and the next
`UserPromptSubmit` corrects the row within seconds. The case this hurts is interrupt-then-
walk-away, which is rarer than the quiet build it fixes. Phase 3's gate check 7 **almost
certainly** still passes — it interrupts a turn that has submitted a prompt, and the first
tool call rarely lands inside the one second before the Esc — but that is a timing
likelihood rather than a guarantee, and if a `PreToolUse` does land in that window the check
fails for this reason rather than a real defect.

### 2.13 A stream, not a library (decision, recorded — added by Phase 5)

Oko's rows are useful to something other than Oko. The first consumer is **panex-tui**, the
terminal build of PanEx (`~/dev/main/panex/crates/panex-tui`), which wants to draw them as
cards behind a keyboard shortcut. Both programs are Rust, and `src/lib.rs` already exposes
`iterm` and `status`, so panex-tui *could* depend on the `oko` crate and call
`Watcher::connect` directly — no subprocess, no serialization, no second API client.

**That is rejected, and the reason is a requirement rather than a preference.** The feature is
meant to degrade to nothing: a person running panex-tui without Oko installed should see no
card view and no error. A compile-time dependency is always present, so there would be no
absence to degrade to. Wanting the feature to be optional is what forces a separate process
and a serialized interface; it is not an implementation detail that happened to be chosen.

So the coupling is **one JSON stream, in one direction**, and the whole contract is two facts:
that a binary named `oko` may be on `PATH`, and the shape of the lines it writes. Neither
program links the other. This is deliberately the same shape as §2.3's coupling with Claude
Code — one directory of small files, one direction — and for the same reason: the failure mode
is a consumer that sees nothing, which is visible, rather than one that sees something wrong.

**Two constraints this design gets for free, and both would have been real problems for the
Tauri build of PanEx rather than the terminal one.** panex-tui runs in an iTerm2 pane, so a
child it spawns inherits `TERM_SESSION_ID` and `src/iterm/watch.rs:resolve_own_session` works
unchanged — the stream is scoped to panex-tui's own window, and §1.1's no-cross-window
non-goal is untouched. And that child sits inside iTerm2's process tree, so macOS attributes
its `osascript` call to iTerm2 and no Automation grant is involved (`rules/iterm-api.md`).
A GUI process spawning Oko from outside iTerm2 has neither property, which is why **this phase
is scoped to panex-tui and a card view in the Tauri app is not in it.**

**Two Oko instances in one window are already safe**, which this design needs and did not have
to arrange: §2.10 turns on two instances seeing one `user.okoName` with no sync protocol, and
OQ-4 (b) scopes the status sweep to the whole `ListSessions` response *precisely* so two Okos
do not delete each other's files. A dashboard tab and a panex-tui card view can therefore run
side by side. That fell out of Phases 3 and 4 without this use case in mind.

**What Oko does not do here.** It does not spawn panex-tui, know panex-tui exists, or change
behaviour when one is watching. §1.1's non-goals hold: the stream observes and reports, and
nothing in it lets a consumer act on a session except by reading. §2.6 rejected a side panel
because a split pane lives in one tab and would need one copy per tab; a separate program
reading a stream is not that, so that argument does not reach this.

**CORRECTED 2026-08-17 (Phase 6).** Two sentences above are now misleading, and in opposite
directions — one understates the contract, the other overstates the conclusion.

- **"the whole contract is two facts"** is three. `--version` is the third, and it is not a
  convenience: a `PATH` lookup cannot tell a build that speaks this stream from one that
  predates it, and the older build answers `--follow` by falling through to the dashboard,
  where `ratatui::init()` meets a pipe and panics. See §2.14.
- **"nothing in it lets a consumer act on a session except by reading"** stays true of *the
  stream* — no request channel was added, no reader became a writer — but is now the wrong
  thing to conclude about *Oko*, which a reader stopping here would. A consumer can act, by
  a second short-lived process rather than through this pipe; §2.14 is the argument, and it
  is what preserves the sentence's literal claim rather than contradicting it.

The paragraph is otherwise untouched, and its §1.1 sentence needs no correction for a reason
worth stating: the two commands Phase 6 adds are `↵` and `r`, which the table already did.
No non-goal moved — **on the reading that §1.1's cross-window non-goal is about the view**,
which is the literal one and is the reading OQ-11 declines to settle. If OQ-11 lands the other
way, §1.1 is what changes, and this sentence is what will be wrong.

### 2.14 Acting is a second invocation, not a second direction (decision, recorded — added by Phase 6)

§2.13 gave a consumer rows it can draw and no way to act on what it drew. That held for
exactly as long as nobody drew them. panex-tui's card view (`Ivapo/PanEx#3`) renders one card
per row and then wants what a human looking at the same rows wants — jump to that tab, name
it — and the only answer Oko had was *go to the dashboard tab and press a key*, which asks a
person to leave the view they are in to act on the view they are in.

**Two ways to fix that, and the difference between them is the direction of the pipe.**

1. **Widen the stream.** `--follow`'s stdin is unused; a consumer could write requests into it
   and the writer could act on them.
2. **A second process per action.** The stream stays one-directional; a consumer that wants to
   act spawns `oko --activate <id>` or `oko --set-name <id> [name]`, which connects, does the
   one thing, and exits.

**(2), and the reason is the same requirement that produced §2.13 rather than a new one.**
One-directionality is not an accident of the pipe, it is what makes the failure mode visible:
a stream that only ever writes fails by stopping, which a consumer sees. Give the reader a
way to write back and the interface acquires request/response — correlation, timeouts, and a
failed command that has to be reported *inside* a protocol whose only rule today is "every
non-blank line is a snapshot". §2.13 bought a contract small enough to state in two sentences;
routing commands through it spends that. And the asymmetry is real in the other direction too:
a consumer that only wants to act — a keybinding that jumps to a session it already knows the
id of — would have to open and hold a stream to send one message.

The cost is stated rather than hidden: **every command pays a fresh connection and a fresh
authorization** (`src/iterm/client.rs:Client::connect` → `request_cookie_and_key`, then
`src/iterm/watch.rs:Watcher::connect`'s `list_sessions`, `resolve_own_session`, `rescan` and
its per-session subscriptions), all to send one request and exit. That is cheap for one
keypress and it is now the interface, which is OQ-10.

**Why these two commands and no others.** `--activate` and `--set-name` are `↵` and `r`,
exactly — the same `src/iterm/watch.rs:Cmd` variants the dashboard has sent since Phases 2 and
4, reaching iTerm2 through the same `src/iterm/watch.rs:Watcher::execute`. Nothing new became
possible; a second door opened onto the same two things. **That claim is only true if the two
doors also refuse the same things**, and one of them did not: the dashboard's editor trims its
buffer and maps empty to `Cmd::Rename(_, None)` (`src/ui.rs:on_key_editing`), so no keystroke
can produce a blank name, while `oko --set-name <id> ""` reached exactly the `Some("")` state
§2.10 calls a trap. `parse_command` now trims and maps empty the same way, and the alignment
is part of this phase rather than a later tidy-up — a door that can reach a state the other
cannot is a second interface, not a second door. That is what keeps §1.1's non-goals
literally intact — no prompt is sent, no permission request is answered, nothing is spawned,
killed or resized — and it is the boundary this section draws for later: **a third command is
a new decision, not an extension of this one.** The test is whether the dashboard can already
do it.

**One implementation, because two would drift.** `Watcher::execute` is public for that reason
rather than for convenience: a rename has to write `user.okoName` *and* patch the in-memory row
(`src/iterm/watch.rs:rename`), and a second implementation that forgot the second half would
look like a flaky rename rather than a missing write. An absent name clears, matching
`Cmd::Rename(_, None)` and OQ-5's measured `null`-unsets — `""` would read back as a name and
render blank, with no further rename able to escape it.

**The join is the session id the stream publishes.** `session_id` is already the first field of
every row (`src/follow.rs:row_json`), so the interface a consumer keys its cards on is the
interface it acts through, with nothing to look up in between.

**Presence is not capability, and that is a third fact in §2.13's contract.** "A binary named
`oko` may be on `PATH`" is what a consumer can check; it is not what a consumer needs to know.
A build predating Phase 5 has that name and does not speak the stream — it treats `--follow` as
an unrecognised argument and falls through to the dashboard, where `ratatui::init()` meets a
pipe and panics. What the caller gets is a dead child and an escape sequence, which is
indistinguishable from a bug in itself. So `--version` answers **ahead of every other branch**
(`src/main.rs:run`), before `--follow` and before any connection.

**CORRECTED 2026-09-02 (added with Phase 7, which changes the second half of this).** The
fall-through described above is a fact about builds up to and including Phase 6, and stays
one — §2.14's argument for answering early is untouched, and gate check 2 measured it. What
changes from Phase 7 on is Oko's *own* behaviour: an unrecognised leading flag becomes a
usage line on stderr and a non-zero exit rather than a dashboard (§2.15). So a consumer
probing a **future** Oko meets a clean refusal instead of a panic, which is strictly the
better failure and does not weaken the case for `--version` — a build too old to know
`--version` still falls through, and that is exactly the build the probe exists to catch.
The sentence is left as written because it describes what those builds do.

**How expensive the fall-through is, stated as what was actually measured.** One `--activate`
against a live window costs ~120 ms (Phase 6, check 10); `--version` on the same build returns
in 2–3 ms. The difference — **113–122 ms** — is *everything a command does beyond printing a
version*, which is the `osascript` cookie, the handshake, `list_sessions`,
`resolve_own_session`, `rescan`'s per-session variable fetch and subscriptions, and then the
one request. **That subtraction does not isolate the connection from the rest**, and saying it
does would matter, because OQ-10's first candidate remedy is a path that skips `rescan`.
A stale Oko pays the whole ~110 ms and then panics, which is
why answering early is what makes a consumer's probe bounded — panex-tui bounds it at 300 ms
and caches the result once per process. The consumer-side figure often quoted alongside this,
that an uncached probe took its suite from 6 s to 173 s, is an **aggregate over an unstated
number of spawns** and is evidence that the cost compounds, not evidence about one call; the
number above is the one about a single call.

**What this does not change.** The stream is byte-for-byte what Phase 5 shipped: no new field,
no schema bump, no stdin. §2.13's "degrade to nothing" still holds end to end, and now covers
the commands too — no Oko on `PATH` means no cards *and* no key that does nothing, which is how
panex-tui already spells it. Two Oko instances in one window stay safe for §2.13's reasons,
with one addition this phase leans on: a rename from a one-shot invocation reaches a dashboard
Oko in the same window through `user.okoName`'s variable notification, which is OQ-5's third
measurement. **Phase 5's check 9 already leaned on it** — a rename in the dashboard appearing
in the stream — so what is new is the *direction*, a writer that draws nothing reaching a
reader that draws, and the writer is gone by the time the reader hears about it.

### 2.15 What Oko is called, and what it says it is (decision, recorded — added by Phase 7)

Six phases produced a tool that runs on one machine because that machine built it. `cargo
install --path .` is the only install path the README knows, and every consumer sentence in
§2.13 and §2.14 — "a binary named `oko` may be on `PATH`" — quietly assumes a human who
cloned this repository. Publishing to crates.io is what makes that assumption false, and
three things in the repo are only correct while it holds.

**The name is taken, and the package name is not the binary name.** `oko` on crates.io is an
unrelated project — a home security system, fifteen versions, owned by `piotrpdev`, last
published 2025-04-30. It is a live crate and not a squat, so this is settled rather than
negotiable: **the package is `oko-iterm2`.** The binaries are unaffected, because a package
name and a binary name are different things — an explicit `[[bin]]` per target keeps `oko`,
`oko-hook` and `oko-probe` exactly as they are typed today.

**The library target is a third name, and it is the one place this rename reaches code.**
`Cargo.toml` carries no `[lib]` section, so cargo derives the library's name from the
package's — the rename makes it `oko_iterm2` and every `use oko::…` in the crate stops
compiling: thirteen references across five files (`src/main.rs`, `src/ui.rs`,
`src/follow.rs`, `src/bin/oko-hook.rs` and the probe) — eleven of them real, two being
doc-comment mentions — producing nine `E0433: cannot find module or crate 'oko'` errors and
failing `cargo check --bins` on all three binaries. Measured 2026-09-02 in
a scratch copy, because "the binaries are unaffected" is the kind of claim that is true of
the binaries and false of the build. So the rename is **two** explicit namings, not one:
`[lib] name = "oko"` beside the three `[[bin]]` entries. **This does not settle OQ-13 by the
back door** — naming the target is what keeps the crate compiling under any of that
question's three answers, and whether the lib is *published with a public surface* is what
stays open. The alternative, rewriting thirteen imports to `oko_iterm2::`, is a larger diff
that decides exactly as little. One consequence to state rather than discover: a downstream
depending on both this crate and crates.io's `oko` would have two libs called `oko` and would
rename one at the dependency line — a cost borne by nobody today, and one more reason OQ-13's
third answer looks like the one it lands on.

`oko-iterm2` is preferred over
`okoterm` or `oko-tui` for the reason the name has to do work it never did before: a
stranger finds this by searching for `iterm2`, not for `oko`, which is a word that means
nothing to them. **What a human types stays `oko`**; what `cargo install` takes is the part
that changed, and it is one line in a README.

**`probe` cannot ship as `probe`.** It was named in Phase 1 as a spike on one machine, and
that name was free because nothing else on that machine wanted it. `cargo install` puts it
in a stranger's `~/.cargo/bin`, where a binary called `probe` is a collision waiting to
happen and is unattributable to Oko when it happens. It becomes **`oko-probe`**, which is
the same rule `oko-hook` already follows. This is the one place Phase 7 changes something a
human types, and it is renamed rather than dropped: §4's Phase 6 gate is built out of it,
and a diagnostic you cannot run is how a gate check becomes unfalsifiable.

**The licence field describes the source and the tarball is not the source.** `cargo package
--list` puts `proto/api.proto` inside the published artifact, and that file is GPL-2.0 and
vendored verbatim (`proto/NOTICE.md`). `license = "MIT"` was true of everything Oko wrote and
was never a claim about what shipping would distribute, because until now nothing shipped.
**The declaration becomes `MIT AND GPL-2.0`**, which is the SPDX expression for what the
tarball actually contains rather than a choice between the two: Oko's source under MIT, the
vendored schema under GPL-2.0. The consequence is stated rather than buried — **anyone taking
a library dependency on this crate takes the GPL-2.0 obligation with it**, which is a cost
this section pays deliberately and which OQ-13 is about.

**Two alternatives, and why neither wins.**

1. **Trim `api.proto` to the messages Oko uses and stay MIT.** Oko touches about fifteen of
   the file's 123 messages and enums, so the trim is bounded, but the envelope oneofs
   (`ClientOriginatedMessage`, `ServerOriginatedMessage`) must keep their tags to stay on the
   wire, and every field number that survives came from the GPL-2.0 file. That reduces the
   volume copied without clearly changing what was copied, which is the wrong trade for a
   phase whose whole job is to stop misdescribing the artifact. **It is also a rewrite of the
   one file `rules/iterm-api.md` treats as the fixed point**, and §2.1 pinned it by commit and
   hash precisely so a schema change would be a visible diff — trimming it makes every future
   iTerm2 diff unreadable against a pinned upstream.
2. **Declare the whole thing GPL-2.0.** Simplest to defend and gives away more than intended:
   it would relicense 3,500 lines Oko wrote to describe a 49 KB file it vendored.

**`--licenses` is that decision in the product, and it is the only reason the flag exists.**
`LICENSE` and `proto/NOTICE.md` are both in the tarball and neither is anywhere a person who
typed `cargo install oko-iterm2` will ever look — `cargo` unpacks to a registry cache and puts
a binary on `PATH`, and the two files stay in the cache. So the licence facts, which this
section has just spent a page getting right, would be *less* reachable after publishing than
they are in a clone. `oko --licenses` prints Oko's MIT line and the `api.proto` notice —
project, commit, hash and GPL-2.0 — and is the whole path by which an installed Oko can answer
"what did I just install". It is deliberately **not** a dependency-licence dump: `cargo
install` builds from source and the manifest already names every dependency, so a generated
list of forty transitive crates would bury the one fact that is genuinely surprising.

**`--help` finishes what `--version` started.** §2.14 established that answering ahead of
`Watcher::connect` is what keeps a consumer's probe bounded, and fixed exactly one flag.
`oko --help` today builds a `Watcher`, opens the alternate screen and draws the dashboard —
and so does `oko --hlep`, and `oko --licenses` before this phase adds it. That is tolerable
for a tool whose only users clone it and read `src/main.rs`; it is the **first thing a
stranger types**, and answering it with a full-screen takeover of their terminal is the
worst first contact this tool could arrange. So: **`--help`/`-h` and `--licenses` join
`--version` in the early block, and an unrecognised leading `--flag` becomes a usage line on
stderr with a non-zero exit** rather than a dashboard. The last clause is the one that
changes shipped behaviour, and §2.14 carries a dated note saying so.

**The rule that keeps this from growing a dependency.** §2.14 said parsing is a scan for the
first recognised flag and that the phase adds no argument parser. That still holds and is now
load-bearing rather than incidental: `clap` would bring a derive macro, a builder API and a
help format Oko does not control, to serve six flags whose entire grammar is "a flag, and
sometimes one or two operands". `src/main.rs:parse_command` stays a hand-written scan, and
the help text stays a string literal — which also means the help text and the README can
disagree, so the gate checks them against each other rather than trusting either.

**What publishing does not change.** No schema bump — `--follow`'s header is
`{"oko":"<version>","schema":1}` and Phase 7 touches neither field's meaning (OQ-14 is about
which version string that becomes, not about the schema). No new capability: §1.1's non-goals
are untouched, the dashboard draws exactly what Phase 6 left, and nothing here is visible in
Oko's own table. **Oko stays macOS-only and stays honest about it** — `src/iterm/client.rs`
shells to `/usr/bin/osascript` and reads `~/Library/Application Support`, so the crate builds
on Linux and cannot work there. That is a `README` and metadata problem, not a code one, and
it is the second thing a stranger needs to learn before the first `cargo install`.

### 2.16 The binaries are the product (decision, recorded — added by Phase 8)

§2.13 turned down a library *for consumers* — "a stream, not a library" — on the argument
that a stream degrades to nothing and a linked dependency cannot. Phase 7 then published a
crate that ships one anyway, because `src/lib.rs` exists and `cargo` publishes what the
package contains. **The same argument reaches the distribution, and OQ-13 is where it was
parked.** It is resolved: the published crate exposes no library surface, and the binaries
are the whole product.

**Two costs were never accepted and would land on a stranger.** The licence: a binary user
takes GPL-2.0 code onto their disk and runs it, which distributes nothing, while a crate
*depending* on `oko-iterm2` links the generated schema into its own artifact — so §2.15's
obligation falls on someone who wrote `oko-iterm2 = "0.2"` and read a one-line description.
And semver: `Watcher::execute` went public in Phase 6 for one reason (§2.14, "two would
drift") and `Client` has been reshaped in three phases. Nothing has ever promised that
surface is stable, and publishing it is how such a promise gets made by accident.

**Removing `pub` is not the mechanism, and this is the fact that makes Phase 8 a restructure
rather than an edit.** A binary target consumes its own package's lib as an *external
crate*, so `oko::iterm` and `oko::status` are public **because all three binaries require
them to be** — eleven `use oko::…` sites across five files. Delete the `pub` and nothing
compiles. The surface is not an oversight to be tightened; it is load-bearing for the build
exactly as it stands.

So the lib target goes, and the shared modules are included by each binary instead — three
compilations of 1,784 lines rather than one, which is the price. `src/iterm/watch.rs`
reaches `crate::status`, and that path keeps resolving inside a binary crate that declares
both modules at its root, so the coupling survives the move untouched.

**The rejected alternative is the one that looks cheaper and is the reason this is written
down.** Marking both modules `#[doc(hidden)]` and documenting no semver promise costs one
attribute and no restructure — and it is OQ-13's second answer, which that question already
called *the weakest of the three, because a doc comment is not what `cargo update` reads*.
The API would still be there, still linkable, still carrying the GPL-2.0 obligation to
anyone who found it. It buys the appearance of the decision and none of it.

**What this does not change.** No behaviour, for anyone: the dashboard, the stream and the
three commands are byte-for-byte what Phase 7 shipped, and §1.1's non-goals are untouched.
Nothing about the *repository* changes either — a contributor still reads one client in one
place. What changes is what `cargo package` contains, which is the whole point and is why
Phase 7 refused to bundle it (its gate would have measured two things at once).

## 3. Open questions

- **OQ-1 — How does a Rust binary reach the iTerm2 API?** **RESOLVED 2026-08-14 by Phase
  1; the mechanics live in `rules/iterm-api.md`.** Candidate 1 works and nothing below it
  was needed. The endpoint is a Unix domain socket at `~/Library/Application
  Support/iTerm2/private/socket`, present only while the API is on. A human enables it
  once at Settings → General → Magic → *Enable Python API*, effective without a restart.
  Authorization is AppleScript: `request cookie and key for app named …` returns a
  single-use pair carried in `x-iterm2-cookie` and `x-iterm2-key`. §2.1's third fact —
  "a per-process authorization step with a cookie and a user-facing confirmation prompt" —
  is half right. The cookie is per process, but the prompt is not: **the only dialog a
  human sees is iTerm2's own *Enable Python API?*, once, when the feature is switched on.**
  No macOS Automation grant is involved, because a client running inside iTerm2 is
  attributed to iTerm2 as responsible process and an app scripting itself needs no grant —
  `tccd` logged zero `kTCCServiceAppleEvents` requests across every connection made on
  2026-08-14. A client started outside iTerm2 would need one. The transport is
  blocking `tungstenite` over `UnixStream` speaking protobuf, with iTerm2's `api.proto`
  vendored and compiled by `protox` — no second runtime, no `protoc`, one binary.
  *(design call — Phase 1 exists
  to answer it)* The seed named "the iTerm2 Python API" and a Rust/ratatui stack in the
  same breath; those do not compose for free. The question has three parts, and the first
  is not optional: **where the API endpoint actually is** (§2.1 establishes only where it
  is *not*), **how a client enables and authorizes against it**, and **which transport
  Oko uses**. Candidates for the third part:
  1. **Speak the API's protocol directly from Rust.** No second runtime, one binary.
     Cost: implementing a client against a protocol whose stability is iTerm2's business.
  2. ~~**A Python sidecar Oko spawns**, using the official library, emitting
     line-delimited JSON on stdout. Cost: a second runtime and a process to supervise,
     and the `iterm2` module is **not installed** in this machine's `python3` today.~~
  3. ~~**Drop the API for AppleScript/`osascript`.** Cheapest to reach; almost certainly
     cannot deliver §2.2's live variables or §2.5's activate-by-session-id cleanly, and
     it is the one candidate that would also invalidate §2.9.~~

  ~~**Try them in that order, and stop at the first that works.**~~ The criterion is
  ordered, not a judgement call: a transport is acceptable only if it can (a) enumerate
  the sessions of another tab, (b) read both variables §2.2 needs, and (c) activate a
  session by id. ~~Among those that qualify, prefer the one needing no second runtime —
  which is the order 1, 2, 3 above. Candidate 3 additionally reopens §2.9, so choosing it
  is a decision to re-argue the stack, not merely a transport call. **If none of the three
  reaches the API, stop and escalate**: the product as designed is not buildable, and that
  is a finding for a human rather than a problem for Phase 2 to inherit.~~ Candidate 1 met
  all three criteria and the ordering never had to be walked; §2.9 stands untouched.

  **Named and rejected as a transport for the table:** `it2getvar`, shipped in
  `/Applications/iTerm.app/Contents/Resources/utilities` and already on `PATH`, reads
  session variables in-band via escape sequences and works with the API switched off. A
  pane can only address *itself* that way, so it cannot enumerate other tabs and is not a
  fourth candidate — but it is the cheapest possible way for Oko to learn its **own**
  session id, and Phase 1 should not rediscover it as a detour.
- **OQ-2 — How does Oko decide a row is a Claude Code tab?** **RESOLVED 2026-08-15, during
  Phase 3's review round: a session is a Claude tab iff the status directory holds a file
  for its iTerm2 session id.** Full stop — **staleness is a property of the status value,
  never of Claude-tab identity** (OQ-4 (c)). A stale row is still a Claude row, still
  labelled `claude`; it is its *status* that stops claiming `working`. Tying identity to
  freshness would have made gate check 6 fail a correct build for most of a run under the
  gate's own `OKO_STALE_AFTER=10s`. What keeps identity honest is deletion, not ageing:
  `SessionEnd` removes the file and OQ-4 (b) sweeps a pane that died without one. The join
  is an
  exact UUID match against `src/iterm/watch.rs:Row.session_id` — no name matching anywhere,
  which is what the argument below demands. Two consequences worth stating because an
  implementer meets both: the `claude` label in the process column is **a literal Oko
  renders for any row carrying a status**, since the status file has no name field and
  §2.2's job name is untrustworthy; and a session that has started but never been prompted
  is a Claude tab from `SessionStart` onward, which is why that event is in §2.3's table
  rather than the three the seed had. *(design call — blocks Phase
  3)* Not by process name. iTerm2 reports the **deepest** foreground job (§2.2), so a
  Claude tab surfaces as `node`, and on this machine a `caffeinate -i claude` child sits
  in the same process group as its `claude` parent. Name-matching cannot be made reliable
  against a subprocess tree the agent is free to change. Proposed resolution, to be
  confirmed in Phase 3's review: **a session is a Claude tab iff a fresh status file
  exists for it**, and the process name is only ever a display value. This inverts the
  seed's design, which checked the name first.

  **CORRECTED 2026-08-14 (Phase 1 measurement).** "Surfaces as `node`" is too specific.
  Of four Claude Code tabs open during the gate, two reported `node` and two reported
  `rust-analyzer-pr` — a rust-analyzer proc-macro server, deeper in the tree than any
  `node`. A fifth non-Claude tab reported `rustup` and then `probe` seconds apart, being
  the same pane at two moments. The corrected claim is the stronger one: **the value is
  whatever happens to be deepest at the instant iTerm2 sampled**, which is unstable within
  a single session, not merely across configurations. Also measured: `jobName` is
  truncated to 16 bytes (`MAXCOMLEN`), so a long name is not even reported in full.
- **OQ-3 — Does the table refresh on a timer, or does the API push changes?** **RESOLVED
  2026-08-14, during Phase 2's review round: Oko subscribes, and does not poll.** Phase 1
  measured every branch of this, so leaving the choice to plan mode would have been
  leaving a settled question open — which is what §4 exists to stop, and what Phase 3's
  scope already says in its own words. The obligations that come with the answer are
  spelled out in Phase 2's scope, and there are **two** subscriptions rather than one:
  per-session, per-variable ones for `path` and `jobName`, and `NOTIFY_ON_LAYOUT_CHANGE`
  for the shape of the window — which is what carries a reorder, and what places a session
  that `NewSessionNotification` can only announce. And a request in flight must not
  discard a notification. The ≤1 s poll floor below is therefore moot, and is kept as the
  record of what the fallback would have cost. *(design
  call — Phase 2)* The API supports variable-change subscriptions, so the push branch
  exists; whether it covers both variables §2.2 needs is what Phase 1's spike observes.
  If Oko polls instead, **the interval must be ≤1 s**, because Phase 2's gate is keyed to
  a 2-second wall-clock bound and a slower interval fails a gate the spec would otherwise
  never have told the implementer how to satisfy.

  **Observed 2026-08-14 by Phase 1** — still Phase 2's call, but it now has measurements
  rather than a branch. Push covers both variables §2.2 needs: `NOTIFY_ON_VARIABLE_CHANGE`
  delivers `path` and `jobName`, and `NOTIFY_ON_NEW_SESSION` and
  `NOTIFY_ON_TERMINATE_SESSION` both fire, so a tab opening and closing is pushed too.
  Two things Phase 2 has to design around. **A subscription is per session and per
  variable**, so a session that appears later is not covered and must be subscribed when
  its new-session notification arrives — which is a fresh way to get a permanently stale
  row, and it is invisible until someone opens a tab. And **`jobName` is poll-driven
  inside iTerm2**: a 5.000 s `sleep` produced its two notifications 5.602 s apart, so the
  push carries roughly 0.6 s of skew. That fits the 2-second bound with room, but it is
  not instantaneous and a gate measured by stopwatch will see it.
- **OQ-4 — What removes a status file when its session is gone?** **RESOLVED 2026-08-15,
  during Phase 3's review round, and it takes three mechanisms because the two candidates
  below answer different questions** — deletion answers "the directory accretes forever",
  staleness answers "a hung session reads `working` indefinitely", and neither covers the
  other:
  1. **`SessionEnd` deletes the file.** It is the one moment that is unambiguous, it exists
     (matchers `clear`, `resume`, `logout`, `prompt_input_exit`,
     `bypass_permissions_disabled`, `other`), and it costs one more line in the same hook.
     Its hooks share a 1.5-second budget, which a file delete is nowhere near.
  2. **Oko sweeps a file whose iTerm2 session id is in no window of `ListSessions`** — the
     *whole* response, not Oko's own window. This is the correction to the original
     candidate 1 and it is not cosmetic: rows are window-scoped
     (`src/iterm/watch.rs:rescan` filters on `p.window_id == self.window_id`), so deleting
     against the row set would destroy the live status of Claude tabs in other windows, and
     two Okos in two windows would delete each other's files continuously. Scoped to the
     full session list, both agree and neither is wrong. This covers a `kill -9`, where no
     hook runs at all. **It runs in `rescan`, not on the status tick**: `rescan` already
     receives the whole `ListSessionsResponse` on every layout change, which is both the
     scope this mechanism needs and within the 2 s check 8 allows, whereas the tick is
     gated on the status directory's mtime and a closing tab writes nothing. **Buried
     sessions are exempt** — `ListSessionsResponse.buried_sessions` sits outside
     `windows[]` and `src/iterm/watch.rs:flatten` drops it deliberately, so "in no window"
     is true of a buried but perfectly alive Claude session.
  3. **`working` goes stale; `waiting` and `ready` never do.** Every status carries a
     timestamp, and a `working` older than `OKO_STALE_AFTER` (default **10 minutes**)
     renders as **`◌ stale`** — a fourth value in the glyph family §1 sketches with three,
     added by this phase and swept into §1 and `CLAUDE.md` at its close-out, and a status Oko shows
     on a row it still labels `claude`. It is needed because a user interrupt (Esc) fires no
     hook at all (§2.3). The threshold is a trade rather than a safe number: `PreToolUse`
     and `PostToolUse` stamp either side of every tool call, so an agent doing things stays
     fresh — but **one quiet 15-minute build or test run goes stale mid-work**, and that is
     accepted because the failure direction is "I don't know" rather than a confident wrong
     answer. **`waiting` is deliberately exempt**: §1's own example is an agent that has
     been waiting twenty minutes, so ageing that out would delete the answer the product
     exists to give. `ready` is exempt because it is legitimately hours old.

  A pane where `TERM_SESSION_ID` is unset — Claude Code in Terminal.app, or under tmux —
  has no iTerm2 identity to join on, so the hook writes **no file at all** rather than one
  nothing can ever match or sweep.

  **CORRECTED 2026-08-15 (Phase 4, §2.12).** Mechanism (c)'s accepted cost — "one quiet
  15-minute build goes stale mid-work" — stops being the shipped behaviour when Phase 4
  lands. A `working` whose hook recorded a tool still in flight ages on a second, longer
  clock instead. The trade above is otherwise unchanged, and the interrupt hole it exists
  for is still open. *(design call —
  blocks Phase 3)* A closed tab leaves its last status behind. Left alone, the directory
  accretes files forever and a crashed session reads as `working` indefinitely.
  ~~Candidates: Oko deletes files whose session id is absent from the API's session list on
  each refresh; or every status carries a timestamp and stale entries render as `unknown`
  rather than as their last value. These are not exclusive.~~
- **OQ-5 — Can Oko set, read and *watch* `user.okoName` on a session that is not its own?**
  **RESOLVED 2026-08-15, during Phase 4's review round: yes, all three.** Measured by
  `src/bin/oko-probe.rs:var_spike` against iTerm2 3.6.11, writing to a session the probe does not
  occupy: the set returns `OK`, the read-back returns the written string, and — the part that
  was not required to pass — `NOTIFY_ON_VARIABLE_CHANGE` on `user.okoSpike` **does** deliver,
  carrying the new value and the session identifier. So §2.10's storage decision stands, and
  the next phase's cross-instance rename is on measured ground rather than inference.
  **A fourth thing was measured that nobody asked for and Phase 4 needs**: setting the value
  to JSON `null` unsets the variable, and it reads back **absent** rather than as an empty
  string. That is the encoding for "clear this name" (§2.10), and it settles a trap noted in
  the same round — a `""` would have decoded through `src/iterm/client.rs:decode_json_value`
  as `Some("")` and rendered a blank name instead of falling back to the derived default.
  *(needs measurement — blocks Phase 4)* §2.10 rests entirely on this and it is unverified.
  The proto supports the shape: `VariableRequest` takes a `session_id` scope with a repeated
  `set` of `{name, value}` and rejects names not beginning with `user.`
  (`VariableResponse.INVALID_NAME`), and `VariableMonitorRequest` takes a bare `name` with no
  restriction to built-in variables. **Three separate things have to hold** and only the
  first is load-bearing for Phase 4 alone:
  1. **Set, on another pane's session.** Phase 1 only ever *read* variables, and only from
     sessions in its own window. Writing to a session Oko does not own is a capability
     nothing has exercised. If this fails, §2.10's storage decision fails with it and names
     fall back to a file — which drags in OQ-4's three mechanisms, and is the reason this
     blocks the phase rather than being discovered during it.
  2. **Read it back**, including for a session that set it before this Oko connected.
  3. **Watch it.** `NOTIFY_ON_VARIABLE_CHANGE` on `user.okoName` is what makes a rename in
     one Oko appear in another without polling. **Phase 4 does not need this** — one Oko
     renaming a row can update its own state directly — so a failure here is not a blocker
     now. It is recorded because it is the load-bearing assumption of the *next* phase, and
     because the measurement costs nothing once (1) is being tested anyway.

  The spike is one `probe` subcommand, and it belongs in the review round rather than in
  implementation: §4's own rule is that a phase must be plannable from the spec alone, and a
  phase whose storage medium is unknown is not.
- **OQ-6 — How long may a tool be in flight before the row is stale anyway?**
  **RESOLVED 2026-08-15, during Phase 4's review round: `OKO_TOOL_STALE_AFTER`, default 45
  minutes**, and the number now has a derivation rather than being the guess this question
  was raised about. Four bounds fix it. It must exceed `OKO_STALE_AFTER`'s 10 minutes or the
  mechanism does nothing. It must exceed the tool calls that actually run long — the case
  §2.12 exists for is a build or test suite, which is minutes to tens of minutes, not hours.
  It must stay far below a working day, because an agent killed mid-tool claims `working`
  until it expires. And **it must not sit on a bucket boundary**, which is the bound that
  chose the final value: at any threshold equal to a rung of §2.11's ladder, that rung fires
  at the same instant staleness does and is therefore unreachable. That is why 1 hour was
  rejected — `◐ working >1h` could never render — and 30 minutes fails it for the identical
  reason one rung down. **45 minutes is off the ladder**, so a long build legibly climbs
  `>5m` → `>10m` → `>30m`, holding the top reachable rung for a quarter of an hour, before
  Oko gives up on it.

  *(That correction was itself a round-2 finding: the first resolution picked 30 minutes and
  justified it with a `>10m` → `>30m` climb that its own boundary argument forbids. Recorded
  because the shape of the mistake — rejecting a value for a reason and then choosing another
  value with the same defect — is worth more than the number.)*

  **Rejected in the same round: rendering the tool's name** — `◐ working (Bash) >10m` — which
  this question originally floated as an alternative to picking any threshold. It is a good
  idea and it is not this phase's: the same round found Phase 4 already visibly larger than
  Phases 2 or 3, the status cell is already widening to carry an age, and a tool name is
  unbounded text in a fixed column. Recorded here rather than dropped, because the argument
  for it — that a human reading the tool can judge plausibility better than any threshold —
  survives the rejection and should be re-raised when the column is next opened.

  *(design call — Phase 4)* §2.12 proposes `OKO_TOOL_STALE_AFTER`, default **1 hour**, and
  that number is a guess rather than a measurement. It is bounded on both sides by cases
  that matter: too short and the mechanism does not fix the thing it exists for, since a
  long test suite or a large build is exactly what runs past `OKO_STALE_AFTER`; too long and
  an agent killed mid-tool claims `working` for most of a working day. ~~Worth asking during
  review whether a second threshold is even the right shape, or whether the tool's *name*
  should render instead — `◐ working (Bash) >10m` says more than any threshold does, and a
  human reading it can judge for themselves whether twenty minutes of `Bash` is plausible.~~

- **OQ-7 — What does the stream do about a change no consumer would draw?**
  **RESOLVED 2026-08-16, during Phase 5's review round: the schema carries `job` only on rows
  that have no status, and the writer suppresses a line identical to the one before it.**
  Both halves are needed and they answer different things. A row carrying a status has a
  `jobName` that is (a) never displayed — `src/ui.rs:render_row` substitutes the literal
  `claude` — (b) already ruled inadmissible as identity by OQ-2, and (c) measured *unstable
  within a single session*: `node` on two tabs and `rust-analyzer-pr` on two others, whatever
  was deepest at the instant iTerm2 sampled. Emitting it would export instability rather than
  information, so such a row carries `claude: true` and no `job`.

  **CORRECTED 2026-08-17 (by OQ-12's measurement). The decision stands and (c) is wrong.** A
  Claude row's `jobName` is not unstable — it is **invariant**. Claude Code spawns its tools
  without handing them the tty's foreground process group, so the value stays the agent process
  for the life of the session: 38 minutes over seven panes with four status transitions, plus a
  deliberate 76-second tool call, produced zero `jobName` events against a control that fired.
  **The evidence cited above never supported (c) in the first place**: `node` on two tabs and
  `rust-analyzer-pr` on two others is variation *across* tabs, and OQ-2's within-session
  evidence — `rustup` then `probe` seconds apart — was explicitly a **non-Claude** pane, where
  the deepest job really does churn. OQ-2 is right where it was written; this resolution
  borrowed it onto rows it does not reach.
  **Why the wrong reason mattered more than the right conclusion**: "unstable" invites
  *stabilise it and publish it*, and "invariant" forecloses it. It had already propagated into
  `rules/follow-stream.md`, `src/follow.rs` and a comment in the consumer's repo before anyone
  measured a Claude pane. All three now say invariant. **That is not the table's
  display rule leaking into the interface**, which is what made the third candidate look
  suspect: it is the interface declining to publish a field its own spec says is untrustworthy
  for those rows. A row *without* a status still carries `job` verbatim, truncation and all,
  because there it is the value and the only one. The suppression rule then covers what
  remains — a field moving and moving back, a snapshot rebuilt identically — and makes the
  stream's quietness a property of the writer rather than a hope about the reader.
  *(design call — blocks Phase 5)* Phase 4 measured this and it is not hypothetical: `Snapshot`
  equality compares `Row.process`, which a row carrying a status never draws — `src/ui.rs`
  renders the literal `claude` — so an iTerm2 `jobName` re-sample emits a snapshot that renders
  identically. **Anything run in a watched pane moves that pane's deepest foreground job**, and
  a shell loop calling `sleep` once a second produced a *pair* of emissions every ~1.9 s — some
  sixty a minute, with nothing changing on screen (2026-08-16, `rules/dashboard-ui.md`). A
  stream inherits that directly: panex-tui spawning Oko is itself an event in a watched pane.
  Candidates, not exclusive: emit every snapshot and let consumers dedupe; suppress a line
  whose serialized form matches the last one sent; or omit `process` from the schema for rows
  carrying a status, which removes the churn at its source but bakes OQ-2's display rule into
  the interface. **The third is the one to argue about**, because it decides whether the
  schema describes what Oko knows or what a table draws.
- **OQ-8 — Does a JSON stream actually make the test harness hermetic, or only cheap?**
  **RESOLVED 2026-08-16, during Phase 5's review round: only cheap. The word does not survive,
  and the pty harness is retired rather than inherited again.** Verified against the code:
  `src/main.rs:run` calls `Watcher::connect` unconditionally and `src/iterm/client.rs:Client`
  is a concrete type, so producing a single line still needs an enabled socket, a cookie and a
  joining pane. A seam that removed that — a trait over the client, or a `Watcher` built from a
  fixture `ListSessionsResponse` — is a refactor of the one component every shipped phase rests
  on, undertaken to test a mode with one consumer. **That is disproportionate and this phase
  does not do it.** What it does instead is cheaper and aimed at where the risk actually is:
  **serialization is a pure function from `Snapshot` to a line**, so the schema, the version
  header, the omission rule and the suppression rule are all covered by ordinary unit tests in
  the crate with no iTerm2 at all, and the live half — connect, subscribe, exit — is what the
  exit gate is for. Phase 4 parked the harness here on a justification that was half wrong;
  recording that is worth more than carrying it to a sixth phase.
  *(answerable from code now — answered during Phase 5's review)* Phase 4 cut the pty harness and
  parked it here with the justification that "a JSON stream makes every assertion cheap and
  hermetic". **Half of that looks wrong and should be checked before it is inherited.**
  Asserting on a line of JSON is certainly cheaper than asserting on terminal bytes, but
  producing that line still requires `src/iterm/watch.rs:Watcher::connect`, a live API, an
  enabled socket and a joining pane — which is exactly the hermeticity objection that cut the
  harness in the first place, and `src/main.rs:run` still calls `connect` unconditionally. If
  the harness is to be hermetic it needs a **seam**, and naming that seam is the real work:
  a trait over the client, a recorded transcript replayed from a file, or a `Watcher`
  constructed from a fixture `ListSessionsResponse`. If no seam is worth its cost, say so and
  keep the tests live-only — but do not carry the word "hermetic" forward unexamined.
- **OQ-9 — How does a consumer discover an Oko it cannot speak to?**
  **RESOLVED 2026-08-16, during Phase 5's review round: one header line per stream, and a
  consumer that does not recognise the schema renders nothing and says so.** The first line
  written is `{"oko":"<crate version>","schema":1}`; every line after it is a snapshot. **Per
  stream rather than per line**, which was the sub-question: the schema cannot change inside a
  stream, because a stream is one process and one build, so a per-line marker would pay bytes
  on every line forever to answer a question that is settled at connect. The worry that "a
  long-lived stream outlives the process that decided to trust it" does not apply for the same
  reason — the stream *is* the process; upgrading Oko does not change what a running one
  speaks, and the next launch presents a new header. A consumer meeting an unknown `schema`
  shows nothing rather than a partial row, which is §2.7's principle one layer out: absence is
  visible, a confidently wrong card is not.
  *(design call — blocks Phase 5)* The schema becomes a published interface the moment a second
  program reads it, and the two versions then drift independently — panex-tui is released on
  its own cadence and `cargo install` is not a coordinated upgrade. Something has to carry a
  version, and something has to decide what a consumer does with one it does not recognise.
  The cheap answer is a first line naming the schema number, and a consumer that shows nothing
  rather than mis-rendering — the same "visible absence beats a confident wrong answer" that
  §2.7 turns on. Worth settling **whether the version is per stream or per line**, because a
  long-lived stream outlives the process that decided to trust it.
- **OQ-10 — Is a fresh connection and authorization per command the right cost, now that it
  is the interface?** *(design call; its measurement was taken 2026-08-17 and is below —
  blocks nothing in Phase 6, and blocks any consumer issuing commands faster than a human
  presses a key)* One
  keypress paying one connection is unarguable. What changed is who presses it: `--activate`
  and `--set-name` are now something a program calls, and each call runs `osascript` for a
  cookie (`src/iterm/client.rs:request_cookie_and_key`), handshakes, lists every session,
  resolves its own, rescans and subscribes — `src/iterm/watch.rs:Watcher::connect` builds a
  complete watcher and then throws it away to send one `ActivateRequest`. And a cookie is
  single-use and spent by the connection that uses it (`rules/iterm-api.md`), so this is not a
  handle something could cache — the reuse question is really "should a command path exist
  that does not build a watcher", which is a different shape.

  **The number is in, and it answers half the question.** Gate check 10, measured 2026-08-17
  against a live window, five runs each: one `oko --activate` costs **116–124 ms** (median
  117), and `oko --version` on the same build **2–3 ms**. That is comfortably a keypress, and
  the cost half of this question closes: **a command per action is affordable, recorded, and
  not to be optimised on suspicion.** (An earlier sample put `--version` at 13–15 ms; that was
  a binary straight off a rebuild and not yet in the page cache. The figure a consumer meets
  repeatedly is the warm one, and either way the *difference* is what this question turns on.)
  **What the subtraction does *not* give is a breakdown**: the
  113–122 ms between the two covers the cookie, the handshake, `list_sessions`,
  `resolve_own_session`, `rescan` and its per-session subscriptions, and the request itself,
  all together. Anyone reaching for the first remedy below needs that split and does not have
  it — measuring it is the first step of acting on this question, not a step that was skipped
  in answering it.

  **What stays open is the shape, and the trigger is a rate rather than a duration.** 110 ms
  is nothing once and is 1.1 s across ten, so a consumer that renames on every keystroke, or
  fans commands across a window of rows, pays it linearly — and Phase 6's own consumer draws
  a card per row. Nothing forces the question today; a card view acting on one selection at a
  time never reaches it. **The trigger to name is a consumer issuing commands faster than a
  human presses a key**, and the answers then are a lighter path that skips `rescan` and its
  subscriptions (a command needs neither), batching several commands into one invocation, or
  telling consumers to debounce. **The one answer this section rules out in advance is a
  daemon** — a long-lived process accepting commands is §2.14's rejected direction wearing a
  different hat, and it would take the three-fact contract with it.
- **OQ-11 — Should a command refuse a session outside Oko's own window?** *(design call —
  blocks nothing in Phase 6; decides whether §1.1's cross-window non-goal is about the view
  or about the writes)* Neither command checks. `src/main.rs:parse_command` takes the id as
  given, and `Watcher::execute` hands it to `client.activate` or `rename`, both of which
  address a session by id with no window scope anywhere in the path. So `oko --set-name`
  will name a session in a window this Oko cannot see, and `oko --activate` will raise one.
  **The two are not the same question, which is the part worth separating before deciding.**
  For `--activate`, crossing windows is arguably the feature — "jump to that session" has an
  obvious meaning wherever the session is, iTerm2's `ActivateRequest` already carries
  `order_window_front`, and refusing would mean a consumer holding a valid id it is not
  allowed to use. For `--set-name`, the write lands where the caller cannot see it: no Oko in
  *that* window need be running, and if one is, it learns the new name through
  `user.okoName`'s notification and shows a row renamed by something in another window
  entirely. That is not obviously wrong — it is how one Oko's rename reaches another one
  today (§2.10) — but it is a capability §1.1 never considered, because §1.1 is written about
  what the *table* shows.
  **The honest position is that this was not decided, it was inherited**: the id-addressed
  path came from the dashboard, where every id came from Oko's own window and the question
  could not arise. Three answers are open — scope both to the own window, scope neither and
  say so in §1.1, or scope `--set-name` and not `--activate` on the asymmetry above. **The
  third is the one that needs the most argument and is therefore the one to be suspicious
  of.**

  **Measured 2026-08-17 (gate check 11), so this is not a reading of the code: both cross.**
  Against a scratch second window, `oko --set-name` returned 0 — and
  `src/iterm/client.rs:set_variable` bails on any status that is not `OK`, so iTerm2 accepted
  a write to a session in a window the caller cannot see — and `oko --activate` moved focus
  into that window. Neither command is scoped anywhere in the path, and the capability is
  live today rather than latent. **The observation does not decide it**, which is the point of
  taking it separately: what it removes is the possibility of settling this by arguing about
  what the code probably does.
- **OQ-12 — Should a row carrying a status publish `job` alongside it?** **RESOLVED 2026-08-17
  by measurement — no — and the question's own premise was false.**

  **No, on stronger grounds than OQ-7 gave.** A Claude pane's `jobName` is not unstable, it is
  **invariant**. Claude Code spawns its tools without handing them the tty's foreground process
  group, so the pane's deepest foreground job stays the agent process throughout. Measured with
  `probe watch`: 38 minutes over seven panes, with four Claude status transitions in that
  window including a `working` carrying a `Bash` tool, plus a deliberate 76-second tool call in
  a watched Claude pane spawning `find` and `python3` — **zero `jobName` events**, against a
  control that fired (`probe`'s own startup moved its pane's job at 0.373 s). So `job` on such
  a row would repeat one constant forever. There is no version of the field that names the
  work, which retires the "does it track the tool or thrash" framing: it does neither.

  **The premise was wrong too, and that is the more expensive half.** This question asserted
  the omission "is why a consumer cannot draw a process and a status on one card". A consumer
  always could. What the consumer wanted beside the status was the constant word `claude`,
  which schema 1 has published since Phase 5 as `claude: true`, and which the dashboard's own
  table derives the same way — `src/ui.rs:render_row` draws the literal `claude` because a
  status file exists, never because of a process name (OQ-2). Nothing was blocked, and the
  card needed a rendering change in the consumer rather than a field here (`Ivapo/PanEx#4`).
  **So: no `schema: 2`, and no phase.**

  Where the correction landed: `rules/follow-stream.md` and `src/follow.rs:row_json` now say
  invariant rather than unstable, and OQ-7's resolution carries a dated `CORRECTED` note —
  it had borrowed OQ-2's within-session evidence, which was measured on a *non-Claude* pane,
  onto Claude rows it does not reach. OQ-2 itself is correct as written and untouched.

  *(design call, and it needs a measurement nobody has taken — blocks a schema 2; ~~it is why a
  consumer cannot draw a process and a status on one card~~)* Today the two are exclusive: `src/follow.rs:row_json`
  writes `claude: true` **or** `job`, never both, and OQ-7 argued that publishing a Claude
  row's `jobName` would export instability rather than information — it is a *descendant* of
  `claude` (`node` on this machine, OQ-2), it is never displayed, and it was measured moving
  within a single session. **That argument is about identity and display, and the consumer's
  question is neither.** panex-tui draws a card, not a row, and a card has room for what a
  table column did not; a human looking at three agent tabs may well want to know that one of
  them is inside a `cargo test` while it says `working`. The exclusivity is what makes that
  undrawable, and it was not chosen for the card case — the card case did not exist.
  **What is genuinely unresolved is whether the field would be information at all**, and that
  is measurable rather than arguable: sample `jobName` on a Claude row over a working session
  and see whether it tracks the tool in flight or thrashes between `node` and whatever the
  agent spawned. If it thrashes, the answer is no on the original grounds and §2.12's
  in-flight tool is the honest place for that signal instead. **A `job` that is sometimes the
  agent's real work and sometimes `node` is the bad outcome**, because a consumer cannot tell
  those apart and would draw the second as confidently as the first — §2.7 again, one layer
  out. Resolving this either way is a schema change and therefore a `schema: 2` and a phase
  of its own; adding a field a consumer must ignore is not a compatible extension when the
  consumer's contract is "a header you do not recognise draws nothing".

- **OQ-13 — Should the library half be published at all?** **RESOLVED 2026-09-02, after
  Phase 7's gate: no — the third answer, and §2.16 is the decision.** The published crate
  exposes no library surface and the binaries are the product, which is §2.13's *a stream,
  not a library* applied to the distribution rather than to the consumer. Phase 8 implements
  it; **the mechanism is what that phase's review round settles**, not this resolution.

  **Two things this question got right, and one it did not.** The costs are as stated — the
  licence lands on a stranger who wrote one dependency line, and the semver promise gets made
  by accident. And the test it named was the correct one: *what settles it is whether anyone
  wants it*, and no consumer has asked in the two weeks since §2.13 turned one down.

  **What it missed is that removing `pub` is not available.** A binary target consumes its
  own package's lib as an external crate, so the surface is public *because the three
  binaries require it* — eleven `use oko::…` sites across five files, and
  `src/iterm/watch.rs` reaching `crate::status`. This question reads as though the third
  answer were a matter of tightening visibility. It is a restructure, and that is why it is a
  phase rather than a line in Phase 7's close-out. *(design call — blocks nothing in
  Phase 7, and decides who inherits §2.15's GPL-2.0 obligation)* The crate has a lib target.
  `src/lib.rs` exports `oko::iterm` and `oko::status`, and it exists because three binaries
  share one client — not because anyone asked for a library. Publishing it makes that internal
  seam a public API on crates.io, and two costs follow that were never accepted. **The first
  is the licence**: a binary user takes GPL-2.0 code onto their disk and runs it, which
  distributes nothing; a crate that *depends* on `oko-iterm2` links the generated schema into
  its own artifact, so §2.15's obligation lands on a stranger who wrote `oko-iterm2 = "0.1"`
  and read a one-line description. **The second is semver**: `Watcher::execute` went public in
  Phase 6 for one reason (§2.14, "two would drift") and `Client` has been reshaped in three
  phases. Nothing in this document has ever promised that surface is stable, and publishing it
  is how such a promise gets made by accident. **Three answers.** Publish it as-is and accept
  both. Publish it with the lib documented as an implementation detail carrying no semver
  promise — honest, and the weakest of the three, because a doc comment is not what `cargo
  update` reads. Or give the lib no public surface at all in the published crate and let the
  binaries be the product, which is §2.13's decision — *a stream, not a library* — applied to
  the distribution rather than to the consumer, and is the answer this question expects to
  land on. **What settles it is whether anyone wants it**: §2.13 turned down a library for
  consumers on the argument that a stream degrades to nothing and a linked dependency does
  not, and no consumer has asked since.
- **OQ-14 — Which version is the first published one?** **RESOLVED 2026-09-02, at Phase 7's
  close-out: `0.2.0`, and the gate is what settled it rather than the argument below.**

  **Check 6 measured the thing this question is about, and the answer was worse than
  predicted.** The old build and the new one, run against the same window, produced captures
  that were **byte-identical — header included**. Not "two artifacts answering to one number"
  as an inference from reading `src/follow.rs:header_line`, but two artifacts that a consumer
  holding both streams cannot separate by any byte either of them writes. `--version` prints
  the same nine bytes from both. That is the collision this question raised, observed rather
  than reasoned about, and it is what tipped a question that had been genuinely balanced.

  **The argument against was real and is what the resolution pays.** `0.2.0` does imply a
  feature release, and Phase 7 adds three flags and a rename that no existing user asked
  for. It is accepted because the asymmetry this question already named is decisive: a
  version can be bumped and cannot be un-published, so the cost of being wrong here is one
  misleading minor number, against a permanent inability to tell a crates.io artifact from a
  local build. **Every `0.2.x` came from crates.io** is a fact worth one overstated bump.

  What it costs, stated exactly: one changed string in a header line
  (`{"oko":"0.2.0","schema":1}`) that `rules/follow-stream.md` documents a consumer as
  ignoring except for `schema`. **No schema bump** — `schema` stays `1`, no row field moves,
  and panex-tui, which has been reading that line since 2026-08-16, is unaffected by
  construction. Check 6 was re-run after the bump and the only difference between the two
  captures is that field, which is what the check permits and now, for the first time, has
  something to permit.

  *(design call — blocks nothing;
  decides one string a consumer may already have parsed)* `0.1.0` is not just the manifest,
  it is the first line of every `--follow` stream (`src/follow.rs:header`, which reads
  `CARGO_PKG_VERSION`), and `rules/follow-stream.md` documents that line as naming the build.
  panex-tui has been reading it since 2026-08-16. **Publishing `0.1.0` reuses a version string
  that already means "a build from Ivapo/oko" for something that now also means "the crates.io
  release"**, so two different artifacts answer to one number and neither `--version` nor the
  stream header can tell them apart. **Publishing `0.2.0` instead** costs one changed string in
  a header a consumer is documented to ignore except for `schema`, and buys a clean boundary:
  every `0.2.x` came from crates.io. The argument against is that `0.2.0` implies a feature
  release and Phase 7 adds three flags and a rename. **This question is cheap to get wrong in
  one direction only** — a version can be bumped and cannot be un-published — so the phase's
  gate records what the header says rather than assuming it.

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
  - A minimal non-TUI binary (`src/bin/oko-probe.rs`) that connects to the API by whichever
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
    connect, resolve Oko's own window, enumerate its sessions, subscribe for changes,
    activate a session by id. **Phase 1 proved the protocol, not the shape.** Its client is
    blocking and one-shot, and Phase 2 is the first code to serve terminal input and socket
    notifications at once, so it needs a reader thread and a channel (or an equivalent).
    Two things Phase 1's `Client::call` does that a Phase 2 client must not: it drops any
    frame whose id does not match the request it is waiting on — which in Phase 2 silently
    eats a notification arriving during an activate or a variable read — and it never
    issues a request after subscribing, so that interleaving has never run.
  - A ratatui table over those rows: tab index (§2.8), process, directory (§1's sketch,
    without the status column). Up/down selection, Enter activates, `q` quits. The process
    column shows `jobName` as iTerm2 reports it, **truncated to 16 bytes** — the column
    displays what the API gives rather than repairing it.
  - Rows track reality: a session opened, closed, split off, **reordered**, or `cd`-ed
    into a new directory is reflected without restarting Oko. **Mechanism: subscribe, per
    OQ-3's resolution** — and it takes two subscriptions, not one, because the per-session
    ones cannot see the table's shape:
    - `NOTIFY_ON_VARIABLE_CHANGE`, per session *and* per variable, for `path` and
      `jobName`. A session that appears later is covered only if Oko subscribes it on
      arrival; getting that wrong yields a row that is permanently stale and looks correct.
    - `NOTIFY_ON_LAYOUT_CHANGE` for the shape of the window, and it is the one that makes
      the `tab` column live. **Dragging a tab creates no session, terminates none, and
      changes no session variable**, so an implementation built only from the bullet above
      shows a stale tab column and nothing tells it to look again. Its payload is a whole
      `ListSessionsResponse`, so the new tab order arrives inside the notification. The
      same path places a session that appears later, because `NewSessionNotification`
      carries a `session_id` and nothing else — no window, no tab — so it says a session
      exists without saying whether it is even in Oko's window.

    **This puts the most weight on the least-measured notification.** Phase 1 saw
    layout-change fire, but its recorded observations of a tab opening and closing are of
    `NOTIFY_ON_NEW_SESSION` and `NOTIFY_ON_TERMINATE_SESSION`; that layout-change also
    covers split and reorder is inference from its payload, not something anyone watched.
    Checks 1, 5 and 7 fail visibly if it turns out narrower, and the remedy is one more
    subscription rather than a redesign — so this is stated to be caught, not to be feared.
  - **What happens when the row set changes under a selection.** Closing the selected row,
    or any row above it, must not leave Enter pointing at a different session than the one
    highlighted: a wrong jump is the failure §2.7 argues is worse than no answer at all.
    Rows are ordered by tab index then position within the tab, and a session missing
    `path` or `jobName` renders as `-` rather than as an empty or omitted row.
  - No status column, no hook machinery, no status files.
- **Exit gate:** **Two** windows, for the reason Phase 1's gate gives — with one window,
  "scoped to my own window" and "listed everything" are the same output, and §1.1's
  no-cross-window non-goal goes untested. The first holds four tabs: an interactive `zsh`
  in `~/dev/main/oko`, an interactive `zsh` elsewhere, `nvim`, and Oko itself. The second
  holds at least two tabs. **Oko is started before any of the checks and restarted for
  none of them** — every check below is a liveness check, which is the whole difference
  between this phase and Phase 1.
  1. Splitting the `nvim` tab into two panes **while Oko is running** produces **five
     rows**, and the two rows from the split tab **share one tab index** (§2.8).
  2. **No row from the second window appears**, at any point during the run.
  3. The `tab` column matches the numbering iTerm2 shows in its **own tab bar** — checked
     after dragging a tab to a new position, which is the case that distinguishes display
     order from creation order and the one property §2.8 leaves unverified. A check that
     the two split rows merely share a number passes for sources §2.8 has ruled out. **If
     it fails, stop and escalate rather than substitute a source**: §2.8 ruled the other
     two out, so a failure here means the API offers no tab numbering that matches what a
     human sees, and what to show in that column becomes a design question rather than an
     implementation one.
  4. Each row's directory equals `pwd` in that pane; each row's process equals `jobName`
     for that pane, by the same rule and the same unambiguous choice of tabs as Phase 1's
     gate. (Not "the basename of the deepest foreground process": that is what `jobName`
     approximates, but it is truncated to 16 bytes, so the two differ for any longer name.
     These tabs are chosen so they do not.)
  5. `cd` in one pane, run a long-running command in another, and close a third tab —
     **not the `nvim` tab and not Oko's own**, which checks 6 and 7 still need: all three
     rows correct **within 2 seconds**, measured by stopwatch from the keystroke.
     Both variables are exercised deliberately — the only latency Phase 1 measured is
     `jobName`'s ~0.6 s of poll skew, and `path` has no recorded number, so this check is
     where it gets one. Record what the stopwatch said.
  6. Enter on the `nvim` row makes that tab the focused tab, and Enter on a row of the
     split tab focuses **that pane**, not merely the tab.
  7. With a row selected, close the tab **above** it: the highlight still names the same
     session it named before, and Enter jumps there rather than to a neighbour.
- **Close-out:** seeds `rules/dashboard-ui.md` (the table, the key bindings, the refresh
  path) and updates `rules/iterm-api.md` for anything the client learned — including
  **re-pointing its `sources` at `src/iterm/`**, and saying whether `src/bin/oko-probe.rs`
  survives the phase. Left alone, that rule regenerates from a throwaway spike and the
  linter cannot see it, because `probe.rs` still exists. It sits close to its cap — which
  has been raised twice already as measurements landed — so the update raises it again or
  cuts, deliberately, and says which. Resolves OQ-3 in §3. **User-facing
  documentation is part of this phase**: Phase 2 is the first phase a human can run, and
  there is no README — a second person needs to know how to start Oko and that the API
  must be enabled once (`rules/iterm-api.md`). Write it or log the gap; §6 allows either,
  and silence is not one of them.

### Phase 3 — Claude Code status from hooks
*Produces the observable: **yes** — it completes it. The status column is the column the
project exists for; Phase 2 is the frame it hangs in.*

- **Scope:**
  - ~~Resolve OQ-2 and OQ-4 during this phase's review round, not during implementation.~~
    Both resolved 2026-08-15, in that round. What they settled is built here.
  - **The hook: a second binary in this crate, `src/bin/oko-hook.rs`.** It reads Claude
    Code's JSON on stdin (`session_id`, `hook_event_name`, and **`notification_type`** —
    without the third, the two `Notification` rows of §2.3's table are indistinguishable at
    the hook and write opposite statuses), reads `TERM_SESSION_ID` — then
    `ITERM_SESSION_ID`, the same two names in the same order as
    `src/iterm/watch.rs:resolve_own_session`, or a pane exporting only the second joins on
    Oko's side while the hook silently writes nothing — and writes one file per iTerm2
    session. A binary rather than a shell
    script because parsing that JSON in shell needs `jq`, which is present here and
    guaranteed nowhere, and because the crate already ships a second binary.
    - Path: **`~/.oko/status/<iterm-uuid>.json`**, absolute — a hook runs with `cwd` set to
      whichever project *that* session is in, so nothing relative and nothing under
      `$CLAUDE_PROJECT_DIR` resolves to this checkout.
    - Contents: iTerm2 session UUID, Claude session id, status, and an RFC-3339 timestamp.
    - Written **temp-file-plus-rename in the same directory**, because Oko reads it
      concurrently. The rename is also what makes the directory's mtime move on every
      write, which is what the reader below watches.
    - Writes nothing to stdout (§2.3), and exits 0 on every path including its own errors.
      A hook that fails must not be visible in someone's Claude session.
  - Registration for **every row of §2.3's table** — that table is the referent, and a
    count restated here is a number that rots — in `~/.claude/settings.json`, which
    has no `hooks` key — with the **absolute** path of the installed binary. Since the
    settings file is outside this repo, `oko-hook --print-settings` emits the exact JSON
    block to paste, and `README.md` documents the step. Oko does not edit that file itself.
  - **Reaching the table** (`src/iterm/watch.rs`, `src/ui.rs`). The shipped program is
    purely event-driven — `src/ui.rs:run` blocks on `events.recv()` and the only producer
    is the watcher — so the status directory needs an event source, and there is none.
    Mechanism: the watcher already wakes every 100 ms to check its command channel
    (`src/iterm/watch.rs:IDLE_TICK`); on that tick it **stats the status directory** and
    re-reads it only when the mtime moved, then emits a snapshot if the merged view
    changed. **This is not the polling OQ-3 ruled out** — that answer is about iTerm2's
    API, which still pushes; this is one `stat` per tick against a local directory, and a
    filesystem-notification crate would be a dependency and a second event source for no
    gain a stopwatch can see.
  - **Where status lives.** The watcher owns a `HashMap<session id, Status>` beside its
    rows, and `Watcher::snapshot` merges it in: `Row` gains a `status` field that is filled
    at snapshot time, so `self.rows` carries `None` there and `rescan`'s `rows != self.rows`
    is left doing exactly the job it does today. **Status changes are therefore caught by
    the tick's own comparison of the merged view, not by `rescan`'s** — one sentence worth
    being precise about, because the two mechanisms look interchangeable and only one of
    them ever sees a status move. `src/ui.rs` gains a fourth column without gaining a
    second source of truth. The columns become `tab · process · status · where`, the status
    cell carrying the glyph and the word as §1's sketch draws them — that sketch's header
    line predates the column and names three, and the phase's layout is what governs. The
    process column renders the literal `claude` for any row carrying a status (OQ-2), which
    is a **deliberate change to `rules/dashboard-ui.md`'s current claim** that the column
    shows `jobName` verbatim.
  - Staleness and deletion, per OQ-4's three mechanisms. `OKO_STALE_AFTER` is read from the
    environment (default 10 minutes) so the gate can exercise the rule in seconds.
- **Exit gate:** **One window, three tabs**: two Claude Code sessions (A and B) and Oko
  itself — Oko's own tab is not optional, and omitting it fails every check for a correct
  build. Oko is **started before the checks and restarted for none of them**, as in Phase
  2, because every check below is a liveness check. Run with `OKO_STALE_AFTER=10s`.
  1. Submitting a prompt in tab A flips A's row to `working` within 2 seconds, and **B's
     row does not change** — the cross-talk check, and the one most likely to fail.
  2. A tool-permission prompt in tab A flips A's row to `waiting`. **Do not answer it
     immediately**: the notification is documented to fire about six seconds after Claude
     stops seeing typing, so a checker who answers in three seconds sees no `waiting` at
     all and fails a correct implementation. This is the one literal here that can do that.
  3. **Approving that permission flips A back to `working` within 2 seconds.** Nothing
     announces a granted permission, so an implementation built from the seed's three
     events reads `waiting` for the rest of the turn — a status that lies, which is what
     §2.7 rejects screen-scraping to avoid.
  4. Turn completion in tab A flips A's row to `ready`.
  5. Driving B through the same transitions moves only B's row.
  6. Both rows read `claude` in the process column, not the descendant job name iTerm2
     reports for them (§2.2, and OQ-2's correction: `node` on two of four tabs measured and
     `rust-analyzer-pr` on the other two — whatever happens to be deepest at that instant).
  7. **Submit a prompt in A and press Esc a second later**, while `working` is still fresh —
     interrupting a turn that has been quiet longer than `OKO_STALE_AFTER` would find the
     row already stale and test nothing. Nothing fires on the Esc, so A's row is left saying
     `working`; within 12 seconds it must stop, and render `◌ stale`.
  8. Closing tab A removes its row, and **within 2 seconds `~/.oko/status/` contains no
     file named for A's iTerm2 session id** — checked with `ls`, because the previous
     wording ("leaves no status behind that could reattach") named no observation and was
     vacuous under the confirmed join key: a reused pane gets a new UUID, so a stale file
     cannot reattach whether or not anything ever deletes it. Every candidate resolution of
     OQ-4, including doing nothing, passed it.
  9. **Leave B untouched for two minutes after its turn ends.** Its row still reads `ready`
     and never flips to `waiting`. This is the only check that sees `idle_prompt` (§2.3);
     checks 1–8 all complete within seconds and none of them looks again at t+60 s.
- **Close-out:** seeds `rules/claude-status.md` (the hook binary and its events, the status
  file path and format, the identity join, deletion and staleness). Updates
  `rules/dashboard-ui.md` for the new column **and for the claim this phase invalidates** —
  it currently says the process column shows `jobName` verbatim, which stops being true for
  a row with a status. Both target rules sit near their caps (`dashboard-ui` 51/55,
  `iterm-api` 93/95) and `max_lines` is a hard linter check, so this close-out **raises or
  cuts deliberately and says which**, as Phase 2's did. `README.md` is user-facing here
  twice over: the hook install step is part of this phase rather than a follow-up, and its
  "What it does not do yet" section is written for exactly this phase and stops being true.
  Updates the `CLAUDE.md` observable line **and §1's own sentence and sketch**, both of
  which name three statuses and neither of which mentions a stale one.

### Phase 4 — what a row says: a name, and how long it has said it
*Produces the observable: **yes** — it is the first phase that makes §1's own sentence true.
That sentence promises "an agent that has been waiting twenty minutes"; the shipped table
shows `● waiting` and no minutes, so the one number the example turns on has never been on
screen. Naming is the same failure in the other column: `where` distinguishes agents only
when their directories happen to differ, and the case §1 exists for is several agents at
once, which is exactly when they often do not.*

- **Scope:**
  - ~~Settle OQ-5 and OQ-6 during this phase's review round, not during implementation.~~
    Both settled 2026-08-15, in that round. OQ-5 was measured by
    `src/bin/oko-probe.rs:var_spike`, which the round added and which stays as a diagnostic;
    `src/iterm/client.rs:set_variable` is the write path it proved and the one this phase
    builds on.
  - **The name** (`src/status.rs` or a new `src/name.rs`, `src/iterm/watch.rs`, `src/ui.rs`).
    `Row` gains a `name` field, filled at snapshot time beside `status` — the same discipline
    §2.10's derived default requires, since a stored default is the bug. Resolution order is
    two lines: the session's `user.okoName` if set, else the last component of `path`, else
    `-`. The `where` column keeps showing the full path; the name does not replace it.
  - **Renaming** (`src/ui.rs`, `src/iterm/watch.rs`, `src/iterm/client.rs`). `r` on the
    selected row opens an inline edit prefilled with the current name; `Enter` commits, `Esc`
    cancels. **This is the first modal state Oko has**, and the whole of the mode is that
    while it is open `q` does not quit, `Enter` does not jump, and `↑↓` do not move the
    selection.
    **The commit path is not `src/ui.rs`'s to walk.** `src/main.rs:run` moves the `Watcher`
    — and with it the only `Client` — into the socket thread, so the UI reaches iTerm2 only
    through `Cmd`. Committing is therefore a new `Cmd::Rename(session_id, name)`, handled in
    `Watcher::run` beside `Cmd::Activate`, which calls
    `src/iterm/client.rs:set_variable` and updates **the watcher's** rows. Updating the UI's
    own snapshot copy instead would show the new name for one frame and lose it at the next
    emission, which looks like a flaky rename rather than the misplaced write it is.
    Committing an **empty** name sets the variable to JSON `null`, which unsets it (OQ-5,
    measured) and returns the row to its derived default — that is the only way back, so it
    is not optional.
  - **The age** (`src/status.rs`, `src/ui.rs`). One function from a `Duration` to a bucket:
    `None` under 5 minutes, then `>5m`, `>10m`, `>30m`, `>1h`. Rendered in the status cell
    after the word, on all four statuses (§2.11). The bucket is part of what
    `src/iterm/watch.rs:emit_if_changed` compares, or a row crossing a boundary never
    reaches the screen — which is the one way to build this that looks right and is dead.
  - **The columns become `tab · name · process · status · where`.** `name` goes left of
    `process` because it is the more specific answer to "which one is this", and the header
    row gains it. The status column widens from `Constraint::Length(10)` to **`Length(14)`**,
    which is the widest cell the ladder can produce: `◐ working >10m` and `● waiting >10m`
    are both 1 + 1 + 7 + 1 + 4 = **14** cells — the glyphs are East-Asian-Ambiguous and score
    1, which the shipped `Length(10)` holding `● waiting` (9) already demonstrates. **Count
    this rather than trusting it**: at 13 ratatui truncates silently, `>10m` renders `>10`,
    and gate check 7 fails against a correct bucket function. `src/ui.rs`'s other widths are
    unchanged.
  - **The tool in flight** (`src/bin/oko-hook.rs`, `src/status.rs`). **`PreToolUse` records
    `tool_name` in the status file; every other event in §2.3's table writes the field empty**
    — §2.12 argues why that total rule is the design and not a shortcut. A status carrying a
    tool ages against `OKO_TOOL_STALE_AFTER` (default 45 minutes, OQ-6) rather than
    `OKO_STALE_AFTER`. Both are read from the environment so the gate can exercise either in
    seconds.
  - **An emission log, for check 9** (`src/iterm/watch.rs`). §2.11's quietness is the one
    property of this phase that a human watching the screen cannot verify — a redundant
    redraw of identical content is invisible. Under `OKO_DEBUG_EMITS`, `emit_if_changed`
    **appends one line to `~/.oko/emits.log`** every time it actually emits.
    **It is a log and not a counter reported on exit**, and that is the whole design
    constraint: check 9 needs two readings sixty seconds apart *from one running process*, so
    an exit-time total would require two runs whose counters both start at zero — comparing
    nothing. Nor is exit a place to report from: `src/main.rs:run` spawns the watcher with
    `thread::spawn` and never joins it, so anything printed as the process tears down races
    teardown and may never appear. A per-emission `eprintln!` is equally unavailable, because
    Oko owns the alternate screen. A file the running process appends to is readable from
    another tab with `wc -l`, at any moment, without stopping anything.
    **The line carries a timestamp and no row content** — `wc -l` is all check 9 reads, and
    logging *what* was emitted is the natural debugging instinct that breaks check 4, which
    greps `~/.oko/` for a row name it expects to find nowhere on disk.
  - No `--follow`, no JSON, no card layout, no panex, **and no pty test harness**. The
    harness was in this phase's first draft and was cut in review: it needs two new
    dev-dependencies, a `tests/` tree that does not exist, and a seam that does not either —
    `src/main.rs:run` calls `Watcher::connect` unconditionally, so a test would need a live
    iTerm2 with the API on and a joining pane, which is a decision about hermeticity this
    phase has no business making. It earned exactly one gate check, which the counter above
    now serves. It belongs with the phase that adds `--follow`, where a JSON stream makes
    every assertion cheap and hermetic.
- **Exit gate:** **One window**: two Claude Code sessions (A and B), a plain `zsh` tab, and
  Oko. Run with `OKO_STALE_AFTER=10s OKO_TOOL_STALE_AFTER=60s OKO_DEBUG_EMITS=1`.
  Oko is started once and restarted for **exactly one check — check 4, which is a restart by
  design and is therefore run last.** Phases 2 and 3 forbade restarts outright because every
  check there was a liveness check; here the central claim of §2.10 is that a name outlives
  the process that set it, and no check that keeps Oko running can distinguish that from a
  `HashMap`. Checks 1–3 and 5–11 are liveness checks and the old rule holds for them.
  **In tab A, pre-approve the tool that check 10 uses** — add it to that session's
  permissions before starting, or run A in a mode that does not prompt. This is not
  incidental: a tool call that raises a permission dialog writes `waiting` (§2.3), `waiting`
  is staleness-exempt (OQ-4 c), and the row would then sit at `● waiting` for the whole tool
  — failing check 10 against a correct build, while testing nothing about the clock it exists
  to test.

  **Every hand-written status file in this gate — checks 7 and 11 — must be written
  temp-file-plus-rename, or `rm`-ed first.** `src/status.rs:Store::refresh` gates on the
  *directory's* mtime, and a POSIX directory mtime moves when an entry is created, renamed or
  unlinked — **not when an existing file's contents are rewritten in place.** So a second
  pass that overwrites a file the first pass created is invisible to the reader, `refresh`
  returns early, and the new value never reaches the table. That is the mechanism §2.3
  documents and `src/status.rs:write` implements deliberately; a checker who edits in place
  records a failure against a correct build.
  1. **Every row shows a name**, and for un-named rows it is the last component of `where`:
     the `zsh` tab in `~/dev/main/oko` reads `oko`.
  2. **The derived default follows a `cd`.** `cd ~/dev/main` in that tab: within 2 seconds
     the name reads `main`. This is the check that fails an implementation which stores the
     default at first sight, and it is the commonest way to get §2.10 wrong.
  3. **A rename sticks, and stops following.** `r` on A, type `api work`, `Enter`. Then `cd`
     in A: the name stays `api work`. **Leave A named — check 4 depends on it.**
     Then, **on B**, `r`, type anything, `Enter`, and `r` again committing an *empty* name:
     B returns to its derived default. Clearing is tested on B rather than on A precisely so
     that A is still named when check 4 runs; doing both on A leaves check 4 asserting a name
     that this check deleted, and nothing between them restores it.
  4. **The rename is on the session, not in Oko.** *Run this last — it is the one check that
     restarts Oko.* Quit Oko and start it again: A still reads `api work`. And the name is
     nowhere on disk: `grep -r 'api work' ~/.oko/` finds nothing. (**Not** "`~/.oko/` is
     empty" — by now it holds A's and B's status files, which is correct and would fail that
     wording. Phase 3's check 8 was rewritten once for the same defect.) This is the only
     check that separates §2.10's storage decision from a `HashMap` in Oko's memory, which is
     why it is worth breaking the no-restart rule for.
  5. **The modal state is really modal.** With the editor open, `q` types a `q` rather than
     quitting, and `Enter` commits rather than jumping to A's tab.
  6. **No age under five minutes.** A row that has just changed status shows the word alone.
  7. **The ladder renders**, checked without waiting an hour. Hand-write status files for
     **the `zsh` tab and Oko's own session** — not A or B, whose live hooks would overwrite
     them mid-check — backdated 6 and 40 minutes, confirm `>5m` and `>30m`; then write the
     same two again backdated 12 and 90 minutes and confirm `>10m` and `>1h`. **The second
     pass must rename or `rm` first**, per the preamble: rewriting those two files in place
     leaves the directory mtime untouched and the new ages never reach the reader. Write them
     **`status: "waiting"`**, which is staleness-exempt, or `OKO_STALE_AFTER=10s` turns every
     backdated `working` into `◌ stale` and the check reads as a failure of the ladder rather
     than of the reader. Age is a pure function of the timestamp, so a hand-written one tests
     the ladder honestly — and check 8 is what proves real timestamps reach it.
  8. **A real agent ages.** Leave B untouched after its turn ends; within ~5 minutes its
     `○ ready` grows a `>5m`.
  9. **Oko stays quiet.** From another tab, `wc -l ~/.oko/emits.log`; leave every session
     alone for 60 seconds with no bucket boundary due; `wc -l` again. **The line count has
     not moved.** Both readings come from the one Oko that has been running since the
     preamble — no restart, which is what the log rather than an exit-time total buys.
     §2.11's quietness is a requirement rather than a preference, so it is a gate check
     rather than a hope, and it is counted rather than watched because a redundant redraw of
     identical content is invisible to a human — which is exactly the failure it exists to
     catch.
  10. **A tool in flight does not go stale.** In A, run the pre-approved tool from the
      preamble for 30 seconds. With `OKO_STALE_AFTER=10s`, A's row stays `◐ working`
      throughout rather than turning `◌ stale` at ten seconds. **Then interrupt the turn with
      Esc *while the tool is still running***, rather than after it finishes: once
      `PostToolUse` lands the field is cleared, and if the turn then ends normally `Stop`
      writes `ready` and nothing ever goes stale. Interrupted mid-tool, the row holds
      `◐ working` past ten seconds — which is §2.12's stated residual, observed.
  11. **The longer clock is a clock.** Hand-write a status for the `zsh` tab carrying a
      recorded tool and a timestamp backdated 90 seconds — **renaming or `rm`-ing first**,
      since check 7 already left a file there and an in-place rewrite is invisible to the
      reader (preamble). With `OKO_TOOL_STALE_AFTER=60s` it
      renders `◌ stale` rather than claiming `working` indefinitely. **An unbounded
      exemption passes check 10 and fails only here**, which is the one thing this check is
      for. (Doing *nothing* — no tool tracking at all — fails check 10 instead.)
- **Close-out:** updates `rules/dashboard-ui.md` (the name column, the age buckets, the modal
  edit, and the emission property check 9 pins down — **stating that an emission is not the
  same event as a visible change**, since `Snapshot` equality compares `Row.process` and
  `Row.path`, either of which can move without altering a rendered cell) and
  `rules/claude-status.md` (the tool
  field, and the second threshold). `rules/iterm-api.md` gains what OQ-5 measured about
  writing and watching `user.` variables — a capability nothing in the API rule currently
  describes, since Phase 1 only ever read. All three sit near their caps, so this close-out
  **raises or cuts deliberately and says which**, as Phases 2 and 3 did. `README.md` gains
  the name and the age, and its status table gains the age column. Resolves OQ-5 and OQ-6 in
  §3. §1's sketch acquires a name column and an age, and the `CLAUDE.md` observable line is
  re-read to see whether it still describes what ships — this phase adds no status value, so
  it may not need touching, and "checked, no change needed" is the answer to record if so.

### Phase 5 — a stream another program can draw
*Produces the observable: **no**, and this is the argument — the second phase of five that
does not, and it is the same argument Phase 1 made. The visible payoff is a card view inside
panex-tui, which is a different repository and a different document; this phase's own output
is an interface and the tests that hold its schema still. That is exactly the shape §3 warns
about — a well-reviewed thing nobody consumes — so the risk is named rather than waved
through: **if panex-tui's card view is never built, Oko carries a JSON mode with no reader.**
Gate check 7 requires the stream to be consumed end to end by something that is not Oko, but
**that check closes the gate, not the risk**: a reader written for the gate proves the
interface is usable, not that anyone wants it. The risk closes when panex-tui reads it, and
that is a fact about another repository which this document cannot assert.*

- **Scope:**
  - **`oko --follow`** (`src/main.rs:run`, and a serializer beside `src/ui.rs`). Newline-
    delimited JSON on stdout. **The branch is taken before `ratatui::init()`**, and nothing in
    this mode touches the terminal: no alternate screen, no key handling, no footer.
  - **The emission point is the one that exists.** `src/iterm/watch.rs:Watcher::run` already
    takes `emit: impl FnMut(Event) -> bool`, and `src/iterm/watch.rs:emit_if_changed` is
    already the single place a change is published. `--follow` supplies a different closure and
    adds no second view-building path. Two things an implementer must get right, both of which
    a plausible reading gets wrong:
    - **`Watcher::run` also takes `cmds: &Receiver<Cmd>` and returns immediately on
      `Err(TryRecvError::Disconnected)`.** There is no UI here to hold the sender, so
      `--follow` must keep one alive for the life of the process or it exits before writing
      anything.
    - **The opening snapshot has no path through `emit_if_changed`.**
      `src/iterm/watch.rs:connect` ends with `self.emitted = self.snapshot()`, so the state at
      connect can never be published as a difference — the dashboard gets it separately, via
      `src/main.rs:run`'s `let initial = watcher.snapshot()`. **`--follow` writes that same
      snapshot as its first data line**, or a panex-tui that spawns Oko behind a shortcut draws
      an empty card view until something happens to move.
  - **Detection that the reader is gone, and what actually bounds it.** `emit` returning
    `false` already stops `Watcher::run`, and a failed write to a closed pipe returns exactly
    that — Rust ignores `SIGPIPE`, so this rests on the write error, which is observed rather
    than signalled. **But `emit` is only called when something changed**, and §2.11 designs
    emissions to happen a handful of times a day, so a reader that closes the pipe and stays
    alive would otherwise leave an orphan holding a socket indefinitely.
    So `--follow` spawns **one thread that writes a bare newline to stdout every five seconds**
    and, when that write fails, calls `std::process::exit` — a closed pipe is detected within
    one interval. Two properties make so blunt an exit correct *here specifically*: this mode
    owns no alternate screen, so there is no terminal state a teardown would have to restore
    (`ratatui::restore` is on the dashboard path only), and `src/main.rs:run` already spawns
    the watcher with `thread::spawn` and never joins it, so an abrupt end is what this program
    already does.
    **The keepalive is deliberately *not* delivered through `Event`, and that is the whole
    point.** A tick routed through `src/iterm/watch.rs:Event` would reach `src/ui.rs:run`,
    whose `terminal.draw` sits **outside** the action match and is therefore unconditional —
    `Action::Redraw` is a no-op arm precisely because the draw is not optional — so the
    dashboard would redraw ten times a second, forever. That is §2.11's stated defect at ten
    times the rate the section rejects, and **nothing in the corpus would catch it**: Phase 4's
    check 9 counts `~/.oko/emits.log`, and `src/iterm/watch.rs:log_emit` sits *after*
    `emit_if_changed`'s early return, so ticks would never be logged and the count would look
    right while the table redrew continuously.
    Keeping the keepalive local to `--follow` means **this phase modifies neither `src/ui.rs`
    nor the dashboard's path through `src/main.rs:run`** — it adds a branch beside it. That is
    why Phase 4's check 9 cannot regress here, and why no gate check below needs to re-run it.
    **This is not the timer §2.11 refuses**, on a stronger footing than a shared tick would
    have had: the thread renders nothing, carries no row content, and exists only in a mode
    that has a pipe. The table §2.11 is about is untouched. It exists because **the stream has
    a failure mode the dashboard does not — a consumer can vanish silently**, where a human
    closing the dashboard closes the process with it. The cost is stated rather than hidden:
    a consumer is woken twelve times a minute, forever, by a program built to sit in a tab all
    day. That is the price of bounding the orphan, and it is paid in one byte.
  - **The schema**, per OQ-7 and OQ-9. A header line `{"oko":"…","schema":1}`, then one object
    per snapshot carrying `window_number` and `rows` built from `src/iterm/watch.rs:Row`:
    `session_id`, `tab`, `name`, `path`, `status`, `age`, and either `claude: true` (a row
    carrying a status) or `job` (a row without one — verbatim, 16-byte truncation and all).
    - **`age` is the bucket, never seconds.** Seconds would make every line differ every second
      and destroy §2.11's quietness at the interface.
    - **`status` is the effective one**, `stale` included, because `src/status.rs:Status::Stale`
      is derived at read time and a consumer given the written value would have to re-implement
      two clocks to get it right.
    - A line identical to the one before it is not written (OQ-7).
    - **`serde_json::json!` builds it**, exactly as `src/status.rs:Entry::to_json` does.
      `Cargo.toml` has `serde_json` and **no `serde`**, and `Row`/`Snapshot` live in the library
      while `--follow` is binary-local, so reaching for `derive` would add a dependency and put
      derives in `oko::iterm` for one consumer's benefit.
    - **Two threads write one stdout, so the handle is never held across writes.** `writeln!`
      on `std::io::Stdout` takes the internal lock per call, which is what keeps a JSON line
      and a keepalive from interleaving; hoisting `stdout().lock()` out of the writing loop is
      an ordinary-looking optimisation that starves the keepalive thread forever and silently
      restores the orphan it exists to prevent.
  - **Errors.** `src/main.rs:main` covers what `run()` returns, which is the pre-connect
    failure gate check 6 tests. A socket that dies **mid-stream** arrives as `Event::Error` in
    the closure instead: `--follow` writes it to **stderr**, never to stdout, and exits
    non-zero. Stdout carries the header, snapshots and keepalives, and nothing else, ever.
  - **Tests**, per OQ-8: unit tests over the serializer, which is a pure function from
    `Snapshot` to a line — schema shape, the version header, the `job`/`claude` omission and
    the suppression rule. **No `tests/` tree, no seam over the client, and the pty harness is
    retired** rather than parked a third time.
  - **The consumer's half is not in this phase.** The shortcut, the card layout, the
    absent-binary behaviour and the child's lifetime belong to panex, a separate repository
    which **is not spec-driven today** — a `CLAUDE.md`, no `specs/`, no `rules/`. Whether it
    adopts the methodology is that repo's decision, recorded here so the boundary is not
    assumed away.
  - No card layout, no rendering, no `--once`, and nothing that lets a consumer act on a
    session. The stream reports.
- **Exit gate:** **One window**: a plain `zsh` tab, a Claude Code tab, an `oko` dashboard tab
  run with `OKO_DEBUG_EMITS=1`, and the tab the stream is read from. **`--follow` must run
  *without* `OKO_DEBUG_EMITS`** — `src/iterm/watch.rs:emits_log` is one fixed path with no pid
  in it, so a variable exported in the shell would have both processes appending to one file
  and check 1 conflating them. Throughout, "a line" means a **JSON** line. Keepalives are empty lines, so
  **`grep -c .` is the count** — it counts lines with at least one character, which is
  exactly the JSON ones. **`wc -l` is the trap**: it counts every newline, keepalives
  included, and a checker carrying Phase 4 check 9's `wc -l` habit records a failure
  against a correct build.
  1. **The stream is quiet.** Start `oko --follow > /tmp/f`, wait for the header and the
     opening snapshot, then **wait a further 10 seconds before starting the clock** — launching
     it moved that pane's `jobName` from `zsh` to `oko`, which iTerm2 re-samples about 0.6 s
     later (OQ-3) and which is a legitimate emission. From there, with every session idle:
     **no further JSON line for 60 seconds**, and **the `--follow` process is still running at
     the end** (`ps`, not inference — a dead process is silent too). "Every session idle" is
     checked, not assumed: `cat ~/.oko/status/*.json` and confirm no `at` is within 60 s of the
     5, 10, 30 or 60-minute marks, and that no row renders `working`. And **do nothing in any
     tab of that window** — anything run in a watched pane moves its `jobName`, which is the
     trap that cost Phase 4's check 9 three runs (OQ-7).
  2. **The stream opens with the state.** The first data line, before anything is touched,
     carries every session in the window with its current name, status and path — not an empty
     `rows`, and not nothing at all.
  3. **The stream is live.** `cd` in the plain tab: one line appears within 2 seconds carrying
     the new `path` and derived `name`. **Record what the stopwatch said** — `path`'s latency
     has been asserted at 2 s since Phase 2 and never written down, and Phase 2's own check 5
     asked for the number and did not get it.
  4. **It agrees with the table.** For every row, `tab`, `name`, `status` and `age` in the JSON
     match what the dashboard draws at the same moment, and `path` matches after applying
     `src/ui.rs:abbreviate_home` — the table renders `$HOME` as `~` and truncates `name` to 16
     cells, so a literal string comparison fails a correct build. (An agreement check, not a
     discriminating one: two Okos reading one `ListSessions` and one status directory agree
     whether or not the second reuses `emit_if_changed`.)
  5. **Ages are buckets.** A row older than five minutes carries `>5m`, not a second count, and
     the line does not change while that row sits inside one bucket.
  6. **Closing the reader stops the writer.** Close the read end while leaving the reading
     *process* alive — this is the case §2.13's "degrade to nothing" produces, and the one a
     kill would not test. **No shell pipeline does this on its own**: use the scripted reader
     check 8 already requires, closing its end of the pipe and staying up. Within **15 seconds** (three keepalive intervals) the `oko --follow`
     process is gone, checked with `ps`, and the dashboard Oko in the same window is still
     running.
  7. **Stdout carries nothing but the protocol.** Every non-blank line of `/tmp/f` parses as
     JSON, and the first is the header. Turn the API off and start again: the failure goes to
     **stderr**, stdout stays empty, exit status is non-zero.
  8. **A real consumer reads it.** Something that is not Oko — panex-tui if it is ready,
     otherwise a scripted reader — consumes the stream and renders or asserts on `name` and
     `status` for every row. See the observable argument: this closes the gate, not the risk.
  9. **A rename crosses the process boundary.** Press `r` in the dashboard tab and commit a
     name: the stream emits a line carrying it. Two processes, one `user.okoName`, no protocol
     between them — §2.10's claim, observed across a boundary Phase 4 never tested.
  10. **An unknown schema is refused.** Hand the consumer of check 8 a header with a `schema`
      it does not know: it reports that and renders no rows, rather than drawing what it can.
- **Close-out:** seeds `rules/follow-stream.md` — a new rule, not a section of
  `rules/dashboard-ui.md`, whose `covers` is the dashboard Oko *draws*. It records the schema,
  the header contract, the omission and suppression rules, the keepalive, the exit behaviour
  and what the stream deliberately omits, and **declares its own `max_lines`** (§8.1).
  `rules/dashboard-ui.md` needs a smaller change than it looks, and getting the reason right
  matters: **its emission paragraph is untouched** — this phase *uses*
  `emit_if_changed` and changes nothing about it, its early return included, so that paragraph
  stays accurate; and `rules/follow-stream.md` owns everything about the stream. What does
  change is that the rule declares `src/main.rs` among its `sources`, and `src/main.rs:run`
  acquires a **second entry point** before `ratatui::init()`. That is the fact to fold in. It
  is at 109/112, so that close-out still **raises or cuts deliberately and says which**, as
  Phases 2, 3 and 4 each did. `README.md` gains the mode and what a consumer is
  promised. Resolves OQ-7, OQ-8 and OQ-9 in §3 — all three answered in this phase's review
  round, so none is carried into implementation. **The `CLAUDE.md` observable line is
  re-read**: this phase adds a second surface for the same observable rather than a new one, so
  "checked, no change needed" is a likely and legitimate answer. Commit plan and reconciliation
  step are stated in the phase's plan.

### Phase 6 — acting on a row from outside the dashboard

*Produces the observable: **no**, and for the third time in six phases — but the argument is
not Phase 1's or Phase 5's, and pretending it is would be the mistake. Those two produced
mechanism and hoped for a consumer. This one was **asked for by a consumer that already
exists**: panex-tui's card view (`Ivapo/PanEx#3`) draws `--follow` today and had no way to act
on what it drew. So Phase 5's stated risk — "if panex-tui's card view is never built, Oko
carries a JSON mode with no reader" — closed on 2026-08-16, and this phase is the first thing
the reader asked for after it closed. The observable is unchanged: a human still gets the
dashboard, and nothing here is visible in Oko's own table. What is new is that the same two
behaviours are reachable by a program, and the visible payoff is again in another repository.*

***Written after the fact, and this is the disclosure rather than a footnote.*** *The work was
built, verified and pushed before this phase existed: `d4605eb` (`--version`) and `7af81ab`
(`--activate`, `--set-name`), open as `Ivapo/oko#2`. That inverts §3's order — a phase is meant
to be planned from the spec and cleared by its own review round before code — and the cost is
specific, not ceremonial: **a review round held over existing code is a weaker gate than one
held over a plan**, because the reviewer is reading an implementation that already works and
the cheapest verdict is to agree with it. Two things bound that, and both are facts rather than
intentions. **Nothing is merged** — the PR is draft, `main` has none of it, so a blocking
finding costs a force-push and not a revert; the phase is genuinely still refusable. And the
decision this round exists to judge is §2.14's, which is an argument about interface direction
that stands or falls on its own and can be read without the diff. The exit gate below is
written to suit the inversion: it is a **check on the build** rather than a guard for an
implementer, and no check in it may be marked from what the commits claim. It had not been run
when this phase was drafted; what running it found is in the review record, and it found a
defect in the gate before it found anything about the code.*

- **Scope** — all of it exists; this states what the review round is being asked to accept.
  - **`--version` / `-V`** (`src/main.rs:run`). Prints `oko <CARGO_PKG_VERSION>` and returns,
    **ahead of the `--follow` branch and ahead of `Watcher::connect`** — the ordering is the
    feature, per §2.14. It touches iTerm2 not at all.
  - **`--activate <session>`** and **`--set-name <session> [name]`**
    (`src/main.rs:parse_command`, then `src/iterm/watch.rs:Watcher::execute`). Connect, run one
    `Cmd`, exit. An **absent** name clears, matching `Cmd::Rename(_, None)`, and so does an
    empty or all-whitespace one — `parse_command` trims and maps empty to `None` exactly as
    `src/ui.rs:on_key_editing` does, which is the one behavioural change this phase makes to
    what the commits already contained, for the reason §2.14 gives. Parsing is a scan for the
    first recognised flag and its operands; there is no argument-parser dependency and the
    phase does not add one.
  - **`Watcher::execute` is new and public**, and is the *whole* structural change: the
    dashboard's command loop in `Watcher::run` now matches `Ok(cmd)` once and delegates,
    instead of one arm per variant. `Cmd::what` supplies the `jump` / `rename` prefix that the
    inlined arms used to write literally, so `Event::Error`'s text is unchanged. **This is the
    one place a dashboard regression could hide** — it is the only edit to a path a human uses
    — and gate check 8 is aimed squarely at it.
  - **`rules/session-commands.md`** is new, per §8's one-job-each split: `follow-stream.md`
    covers the stream, and acting is not the stream. `follow-stream.md` gains the `--version`
    clause and a pointer to it. `README.md` gains the two commands and the version line.
  - **Not in scope, and each is a decision rather than an omission:** no third command (§2.14's
    test is whether the dashboard already does it); no batching; no stdin, no request channel,
    no daemon; no window scoping (OQ-11 records the behaviour and does not decide it); no
    schema change (OQ-12 would be `schema: 2` and a phase of its own).
- **Exit gate.** **One window**: a plain `zsh` tab, a Claude Code tab, an `oko` dashboard tab,
  and the tab commands are issued from. Every check below is run against the built binary; none
  is inferred from the diff, and **the binary is rebuilt first** — a `target/debug/oko` left
  over from before these commits answers check 1 by drawing a dashboard, which is a failure
  against a build that passes. Checks 1 and 9 need no iTerm2 window and no API. **Check 2 needs
  both**, which is the opposite of the obvious reading and is the trap in it.
  1. **`--version` answers on a pipe.** `oko --version | cat` prints one line, exits 0, and
     leaves the terminal usable; `-V` likewise. Repeat with the iTerm2 API **switched off**:
     identical result — it is ahead of the connection, so nothing about iTerm2 can change this
     answer. **Switch the API back on before check 2**, which is not a courtesy to the next
     check but a precondition of it.
  2. **The build it is meant to distinguish actually fails the other way.** Two builds, one
     flag each, because the claim has a historical form and a present one:
     - **`412cfca`** — the last commit before `f80cc1e` added `--follow` — built and run as
       `oko --follow | cat`. This is §2.14's literal claim: an unrecognised flag falls through
       to the dashboard.
     - **`main`** built and run as `oko --version | cat`. `main` **has** `--follow` (Phase 5
       shipped there), so it cannot exhibit the fall-through for that flag; `--version` is the
       flag it does not know, and it is also the exact probe a consumer runs today.

     Both must panic inside `ratatui::init()`. **The API must be on and the pane must be a
     real iTerm2 pane for either run**, and getting this backwards is how a correct build gets
     failed: `412cfca:src/main.rs:run` calls `Watcher::connect` on its **first line**, before
     `ratatui::init()`, so with the API off — or from anywhere without `TERM_SESSION_ID` — it
     exits 1 with a clean message and never reaches the panic. **If both panic, §2.14 stands.
     If either fails cleanly instead, check the API is on before recording anything**; a
     genuine clean exit with the API on means §2.14 is wrong and the finding is blocking, the
     correction being that `--version` is worth having for a smaller reason.

     **Build both in a `git worktree`, not by moving this one.** Checks 3 onward need the
     phase's binary back; a checker who reaches them with `412cfca` still checked out is
     testing the build that has none of this.
  3. **`--activate` jumps from outside.** From the plain tab, `oko --activate <id>` for the
     Claude tab's session id: iTerm2's focus moves there, exit status 0, stdout empty.
  4. **`--set-name` sets, and the dashboard sees it.** With the dashboard tab running,
     `oko --set-name <id> "api work"` from another tab. The dashboard's row shows the new name
     **without a keystroke in it**. Two processes, one `user.okoName`, no protocol between them
     — OQ-5's third measurement, in the direction Phase 5's check 9 did not test: the writer
     here draws nothing and has exited before the reader hears about it.
  5. **An absent name clears, and so does an empty one.** `oko --set-name <id>` on that same
     row: it falls back to the derived default, the last component of `path`. Then
     `oko --set-name <id> ""` and `oko --set-name <id> "   "`: **both must also clear**, not
     render a blank name. Seeing the derived value come back is itself the proof the variable
     was *unset* rather than set to something empty — `Some("")` renders as a blank, so the
     derived value cannot be what a blank produces.
  6. **A failure is a failure.** `oko --activate NOSUCHSESSION`: **non-zero** exit, one line on
     **stderr**, **nothing** on stdout. Same for `--set-name` with a bad id, and for
     `--activate` with no operand at all. **Record the exact strings** — `rules/session-commands.md`
     quotes one, and a gate that pins only "one line on stderr" is how it came to quote a line
     the code does not produce.
  7. **The stream and the commands join up.** Run `oko --follow > /tmp/f`, take a `session_id`
     from a data line, `oko --set-name <that id> "from a card"`, and confirm `/tmp/f` gains a
     line carrying the new `name`. This is the end-to-end claim of §2.14 — the id a consumer
     draws is the id it acts through — and it is the only check that tests both halves at once.
  8. **The dashboard is not collateral damage.** In the dashboard tab: `r`, rename, commit —
     the name changes. `↵` on another row — focus moves. Then force an error the refactor could
     have mangled, and force it **deterministically**: press `r` on a row, close that row's tab
     while the editor is open, then commit. `src/ui.rs:Edit` holds the `session_id` it captured
     and a layout change does not close the editor, so the rename is sent against a session
     that is gone and the footer must read `rename failed: setting user.okoName on … failed:
     SessionNotFound` (`proto/api.proto:VariableResponse.Status.SESSION_NOT_FOUND`, which
     `src/iterm/client.rs:set_variable` turns into the `bail!`). **Give the close a few seconds
     first**: iTerm2's undo-close grace may keep a just-closed session addressable, and a
     `VariableResponse` of `OK` means no error reaches the footer at all — which is a failure
     recorded against a correct build, the same shape as this gate's own check-2 defect. If the
     footer stays quiet, wait past the undo window and retry rather than recording anything.
     (The obvious version of this — `↵` on a closed row — is a race and not a check: a closing
     tab emits a layout change at once and `src/ui.rs:App::apply` remaps the selection within
     about a second, so the window to press it in is sub-second. Use it only as a bonus, and it
     is the only way to see the `jump` prefix — this check forces `rename` alone. Both come
     from `Cmd::what`'s one match, which is the line the refactor introduced, so seeing either
     exercises it.)
  9. **`cargo test` passes and `cargo clippy` is clean**, including **unit tests over
     `parse_command`** — the phase's only code change, and a pure function, so the trim, the
     three clearing spellings, the missing-operand error and the recognise-nothing case are
     cheaper to hold here than in check 5. Phase 5's serializer tests remain the regression
     surface for the stream half of check 7.
  10. **Record the cost of a command (OQ-10).** Time one `oko --activate` end to end against a
      live window — `time`, three runs, record the numbers here and in the review record. **This
      check cannot fail**; it produces the number OQ-10 needs and nothing else.
  11. **Record the cross-window behaviour (OQ-11).** Open a second iTerm2 window, take a session
      id from it, and run both `oko --set-name <id> "other window"` and `oko --activate <id>`
      from the first. Record what each does. **This check cannot fail either** — it is an
      observation, and OQ-11 is the decision. What it must *not* do is get quietly fixed here.
- **Close-out:** the reconciliation is unusual because most of it already landed with the code,
  so the step is **re-reading what exists against what this round changed**, not writing it
  fresh. `rules/session-commands.md` exists at 44/55 lines — it is checked against the
  round's outcome and against checks 3–8, and it is the file that must absorb anything OQ-11
  turns into. It is at **48/55** lines after the review round corrected it: it had quoted a
  one-shot failure as `oko: jump failed: …`, where `Cmd::what`'s prefix is applied in
  `Watcher::run` and never on this path, so the real string names the operation. That is the
  case for re-reading a rule that landed with its code rather than assuming it —
  **a rule written alongside an implementation is not thereby checked against it** — and check
  6 now records the strings so the next drift is caught by the gate.
  `rules/follow-stream.md` is at 85/85, exactly at its cap, so
  **any further clause there raises the cap or cuts a line, deliberately, and says which** —
  the same rule Phases 2, 3, 4 and 5 each met. `rules/dashboard-ui.md` is the one nobody wrote:
  it declares `src/main.rs` among its `sources` and its "How rows stay true" paragraph says
  `run` has **a** second entry point, naming `--follow`. `run` now takes **three** branches
  before the dashboard — `--version`, `--follow`, and `parse_command`'s pair — and that
  sentence is *incomplete* in precisely the way round 4 of Phase 5 corrected it into being
  complete. **One clause and a pointer to `session-commands.md`**, taken as a net one line: it
  fit the headroom, so neither a raise nor a cut was needed, and the file now sits at
  **112/112**. The next change there is the one that raises or cuts, and it inherits that
  choice from this phase rather than meeting it fresh. `README.md` is re-read, not
  rewritten. **The `CLAUDE.md` observable line is re-read and the expected answer is "no
  change"** — the observable ends "jumping focus to the row you press Enter on", and this phase
  adds a caller for that jump rather than a new thing a human sees; if the round disagrees, the
  line changes and the reason is recorded. Raises OQ-10, OQ-11 and OQ-12 and resolves none —
  though not for the same reason each: checks 10 and 11 were run in the review round and their
  numbers are folded into the first two, closing OQ-10's cost half and putting OQ-11's
  cross-window behaviour beyond argument, while both keep a decision open. OQ-12 needs a
  measurement this phase does not take.
  Commit plan: **one push to `feat/cli-entry-points`** — the spec and review-record commit on
  top of the two that exist, plus whatever the round's findings and the reconciliation require
  — and the PR leaves draft when the round converges, which is the gate the push is the unit
  for.

### Phase 7 — publishing it: a crate a stranger can install, and what it says it is

*Produces the observable: **no**, and the argument is a fourth distinct one — the first three
were Phase 1 (mechanism, hoping for a consumer), Phase 5 (a stream, hoping for a reader) and
Phase 6 (a reader asked). This phase produces **no new behaviour for anyone who already has
Oko**: the dashboard draws exactly what Phase 6 left, the stream is byte-identical, and a
`git pull` here changes five flags across three binaries, one binary's name, and what an
unrecognised flag does. What it produces is a **second
population** — people who never clone this repository — and the observable is unchanged for
them by construction: the whole point is that `cargo install oko-iterm2 && oko` reaches the
same live table §1 describes. **The risk that makes this worth stating rather than assuming**
is that the phase is entirely metadata, documentation and argument handling, which is exactly
the shape of work that can be done thoroughly and still leave the install broken; so the gate
below is built almost entirely out of running the published artifact rather than reading it,
and check 1 deliberately installs from a packaged tarball rather than from this directory.*

- **Scope.**
  - **`Cargo.toml`** — `name = "oko-iterm2"`; `license = "MIT AND GPL-2.0"`; three explicit
    `[[bin]]` targets (`oko` → `src/main.rs`, `oko-hook` → `src/bin/oko-hook.rs`, `oko-probe`
    → `src/bin/oko-probe.rs`) so the binary names survive the package rename, and
    **`[lib] name = "oko"`** so the crate still compiles (§2.15); `rust-version` set to
    **the maximum of the edition floor and every dependency's declared `rust-version` in the
    locked graph**, which is the quantity `cargo install` has to honour and is **1.88.0**,
    not edition-2024's 1.85.0 — `ratatui 0.30.2`, `time 0.3.55` and `darling 0.24.0` each
    declare 1.88.0 (measured 2026-09-02 by `cargo metadata` over `Cargo.lock`). **The
    edition floor is the wrong quantity and understating it is the failure that matters**: a
    declared 1.85.0 promises a toolchain on which this does not build, and nothing on this
    machine notices, because it has exactly one toolchain (stable 1.97.1) and every check
    below would run on it. Check 9 is what makes "verified against a toolchain" executable
    rather than a wish. `homepage`/`documentation` pointing at the repository. The comment above `[dependencies]`
    that says to read `proto/NOTICE.md` before publishing anywhere is **rewritten, not
    deleted** — it becomes the record that the notice was read and what it decided (§2.15).
  - **`src/bin/probe.rs` → `src/bin/oko-probe.rs`** (§2.15). A rename and nothing else: no
    change to what it does — but it is **five strings, not three**. The `//!` usage block's
    three lines, and then `main`'s `eprintln!("probe: {e:#}")` and `run`'s
    `usage: probe activate <session-id>`. Those last two are the ones the phase is actually
    about: §2.15 renames the binary because a stray `probe` is unattributable to Oko when it
    collides, and a binary that still says `probe:` when it fails keeps precisely that
    defect. Its `ADVISORY_NAME` is already `oko-probe` and does not move.
    **Every citation of the old path moves with it.** Three places cite it with a
    `:var_spike` suffix — `rules/iterm-api.md`, §3's OQ-5 and Phase 4's scope — and those are
    corrections of where a file lives, not rewrites of what those passages decided. Check 8
    is what catches the one that gets missed.
  - **`src/main.rs:run`** — the early block gains `--help`/`-h` and `--licenses` beside
    `--version`, and gains a final arm: a leading argument starting with `-` that nothing
    recognised is a usage line on **stderr** and exit **2**, never a dashboard. The help text
    and the licence text are string literals; `parse_command` stays a hand scan and **no
    argument-parser dependency is added** (§2.15).
    **Exit 2 does not come back through `run`'s `Result`.** `src/main.rs:main` maps every
    `Err` to `eprintln!("oko: {e:#}")` and `exit(1)`, so a refusal returned as an error would
    be exit 1 wearing an `oko: ` prefix that a usage line should not have. The arm calls
    `std::process::exit(2)` itself; 2 is the usual usage-error convention and is stated here
    rather than derived from anything.
    **Two consequences of keeping the scan shape, stated rather than left to be discovered.**
    `run` tests `--version` and `--follow` with `std::env::args().skip(1).any(…)` and
    `parse_command` scans for the first recognised flag *anywhere*, so the new arm fires only
    when **nothing** was recognised and the first argument begins with `-`. `oko --hlep
    --version` therefore still prints a version, and a non-flag operand — `oko notaflag` —
    still draws the dashboard. Both are the shape Phase 6 shipped; this phase narrows
    neither, and says so because an implementer reading "an unrecognised leading flag" would
    be right to wonder.
  - **`oko-hook --help` and `oko-probe --help`** (`src/bin/oko-hook.rs:run`, and the probe).
    §2.15's argument is about what a stranger types first and it is not confined to `oko`.
    `oko-hook` matches only `--print-settings` and then blocks in `read_to_string` on stdin,
    so `oko-hook --help` **hangs** — a worse first contact than the dashboard takeover this
    phase exists to fix, because nothing on screen says what happened. **The probe does not
    fall through** — `run`'s `Some(other)` arm already `bail!`s — but it answers with
    `probe: unknown command "--help"; …` and **exit 1**: no usage text, the wrong exit code,
    and the un-renamed prefix this phase is otherwise removing. Each gains `--help`/`-h`
    printing its own usage to
    stdout and exiting 0: for `oko-hook`, the two things it does; for the probe, its `//!`
    block's three lines. **Decided here rather than left to check 2**, which named a
    fall-through as a finding and would therefore have produced one by construction.
  - **`src/lib.rs`** — its module doc names `probe`; that is one word.
  - **`README.md`** — `cargo install oko-iterm2` as the primary install path, with
    `--path .` kept for contributors; the three binary names; a macOS-only line where a
    stranger meets it rather than at the bottom; the licence paragraph reconciled with
    `MIT AND GPL-2.0`; and **the flag list reconciled with the help text in both
    directions**. It documents `--follow`, `--version`, `--activate` and `--set-name` today
    and **not `-V`**, so check 4 fails against a correct help text until it gains one.
  - **Not in scope, each a decision rather than an omission:** no CI release workflow, no
    Homebrew tap, no `cargo-dist`, no man page — those are how you publish *repeatedly*, and
    this phase is about publishing *correctly* once. No dependency-licence dump in
    `--licenses` (§2.15). No lib-surface change: **OQ-13 is raised by this phase and not
    settled in it**, because the answer changes what `cargo package` contains and would make
    the gate below measure two things at once. No `cargo publish` in the gate — the gate ends
    at a verified artifact, and pressing publish is irreversible and belongs to a human.
- **Exit gate.** Checks 1–4, 8 and 9 need no iTerm2 and no API; only 5–7 need a real
  window — check 8 is `cargo` and the linter, which run headless. **Nothing in
  this gate may be marked from reading a diff**, which matters more here than in any previous
  phase because every artefact under test is a file whose correctness is a claim about a
  machine that is not this one.
  1. **The tarball installs and runs somewhere this repo is not.** **Two things first, and
     each is how this check otherwise misreports.** `~/.cargo/.crates.toml` has package `oko`
     owning `oko`, `oko-hook` and **`probe`** today, all three live in `~/.cargo/bin` — so
     installing `oko-iterm2` either refuses (`binary `oko` already exists`) or, forced,
     leaves `probe` behind owned by the old package, and "**`probe` does not appear**" then
     records a failure against a rename that worked. **`cargo uninstall oko` first.** And
     **`~/.cargo/bin/oko` is the only pre-Phase-7 build on this machine** and check 6 needs
     one to diff against, so copy it aside before installing — or build that side in a
     `git worktree` at `ac0bdc7`, which is what a second person on another machine has to do
     regardless and is the reproducible form of this check.
     Then `cargo package`, then
     `cargo install --path target/package/oko-iterm2-<v>` — *not* `--path .`. All three
     binaries land in `~/.cargo/bin` as `oko`, `oko-hook`, `oko-probe`; **`probe` does not
     appear**; `oko --version` from a directory outside this repository prints one line and
     exits 0. Uninstall afterwards, and record whether `cargo uninstall oko-iterm2` removes
     all three.
  2. **`--help`, `--licenses` and a typo each answer without a terminal takeover.** Each of
     `oko --help | cat`, `oko -h | cat`, `oko --licenses | cat` prints to **stdout**, exits
     **0**, and leaves the terminal usable — run every one on a pipe, because a pipe is the
     failure `ratatui::init()` produces and a bare terminal would hide it. Then `oko --hlep`:
     **stderr**, **nothing on stdout**, exit **2**, and a usage line naming the real flags.
     Repeat all four with the **iTerm2 API switched off** — identical results, since all of
     them are ahead of the connection. The `--hlep` line carries **no `oko: ` prefix**: that
     prefix belongs to `src/main.rs:main`'s error path, which exit 2 deliberately does not
     take, so seeing it means the refusal came back as an `Err` and is exit 1 in disguise.
     Then `oko-hook --help` and `oko-probe --help`: each prints its own usage to **stdout**
     and exits **0**. Run `oko-hook --help` **without redirecting stdin** — before this phase
     it blocks in `read_to_string`, so its failure mode is a hang rather than wrong output,
     and a checker who feeds it `</dev/null` sees an EOF error instead and never meets it.
  3. **`--licenses` says the thing `LICENSE` and `NOTICE.md` say.** Its output names MIT for
     Oko's source, and for `api.proto` names iTerm2, the commit `f4ca0004`, the sha256 and
     GPL-2.0. **Diff those five facts against `proto/NOTICE.md` by eye and record it** — the
     literal is a copy and copies drift, which is the same failure `rules/session-commands.md`
     shipped in Phase 6 and which check 6 there now guards.
  4. **`--help` and the README do not disagree.** Every flag in the help text appears in
     `README.md`, and every flag the README documents appears in the help text. There are
     six, in eight spellings (`--version`/`-V`, `--help`/`-h`, `--licenses`, `--follow`,
     `--activate`, `--set-name`), and the check is a list compared by hand, because §2.15 chose
     a string literal over a parser and this is the cost of that choice, paid on purpose.
     **The comparison is over `oko`'s own flags**, not every flag string in the file:
     `oko-hook --print-settings` is documented in the README and will never be in
     `oko --help`. The direction that actually bites is the other one — no `-V` in the README
     today (scope, above).
  5. **The dashboard is not collateral damage.** With the installed binary, in a real window:
     `↑↓`, `↵` moves focus, `r` renames and commits, `q` quits and restores the terminal.
     The early-block edit is the only change to a path a human uses, and this is the check
     aimed at it.
  6. **`--follow` is byte-identical to Phase 5's, except the version string.** Run the old
     build and the new one against the same window, capture both, and diff. **The only
     permitted difference is the `oko` field of the header line**; `schema` stays `1` and no
     row field changes. Record the version string the header carries — that is OQ-14's
     evidence and the reason it cannot be settled by argument.
  7. **`oko-probe` still works, under its new name.** `oko-probe` prints identity and the
     window's sessions; `oko-probe watch` prints notifications. Phase 6's gate is built out of
     this binary, so a rename that broke it would take a previous phase's falsifiability with
     it.
  8. **`cargo test`, `cargo clippy` and `cargo package` are all clean**, and **`spec-lint
     --strict` passes** — the flag *is* the check rather than decoration. An unresolvable
     citation is a **warning**, `.spec-lint.yaml` sets no `unresolved_path_severity`, and
     plain `spec-lint` prints the warning and still **exits 0**: run without `--strict` this
     check passes over exactly the breakage it exists to catch. Three places cite the probe's
     old path with a `:var_spike` suffix — `rules/iterm-api.md`, §3's OQ-5 and Phase 4's
     scope — and the linter is what finds whichever one the rename missed. Also assert the
     target names from `cargo metadata`: **one lib named `oko`**, three bins named `oko`,
     `oko-hook`, `oko-probe`. That is the cheap form of the failure §2.15 measured, and it is
     one command rather than a build.
  9. **The declared floor is a floor.** Re-derive it — the maximum `rust-version` over the
     locked graph — **comparing versions as numbers, not strings**: most of the graph
     declares two components (`instability 0.3.13` is `"1.88"`, and it ties the maximum), so
     a lexicographic max answers a different question. Confirm the manifest agrees; then
     **`rustup toolchain install 1.88.0`
     and `cargo +1.88.0 check --bins`**, which must succeed. This is the check that makes
     "verified against a toolchain, not guessed" mean something on a machine with one
     toolchain, and it is the only way an understated floor is visible here: every other
     check runs on stable 1.97.1, where 1.85.0 and 1.88.0 are indistinguishable. If the
     build fails, the floor rises to whatever it needs and the number is recorded, not
     argued.
- **Close-out.** **Reconciliation, and two of these are forced rather than discretionary.**
  `rules/session-commands.md` (48/55) gains `--help`, `--licenses` and the unknown-flag
  refusal — it is the file that owns what the CLI spells, and it has the headroom. **Its
  `covers` line widens in the same pass, and that is not bookkeeping**: it says today that
  the file covers *the two things Oko does to a session*, which a help text, a licence notice
  and a refusal are not, and `covers` is the regeneration target (§8.1) — text outside it is
  text the next `/sync-rules` drops as unmanaged.
  `rules/dashboard-ui.md` is at **112/112**, exactly at its cap, and its "Three branches sit
  ahead of all of it" sentence becomes wrong the moment `--help` lands. Phase 6's close-out
  named this: *"The next change there is the one that raises or cuts, and it inherits that
  choice from this phase rather than meeting it fresh."* **This is that phase, and it must
  raise or cut deliberately and say which** — the expected answer is a raise, because the
  sentence needs a clause and nothing in that file is redundant, but the choice is made in the
  round and not here. **`rules/claude-status.md` (121/124) is the one this round added**: it declares
  `src/bin/oko-hook.rs` among its `sources`, its `covers` reaches "the hook binary", and it
  already documents `oko-hook --print-settings` — so the hook's new `--help` lands inside its
  regeneration target and belongs there rather than in `session-commands.md`. Three lines of
  headroom, so no raise-or-cut. `rules/iterm-api.md` (113/115) changes one citation path and
  nothing else — and it is not the only file that does: §3's OQ-5 and Phase 4's scope carry the same
  `src/bin/oko-probe.rs:var_spike` citation, and all three move together. **Those two are
  corrections of where a file lives, not edits to what a shipped phase decided**, which is
  the distinction that keeps an append-only document honest while its citations stay
  resolvable (§5). Check 8 is the backstop. `rules/follow-stream.md` (85/85) **does need one, and check 6 is not what finds it**: the
  file names `oko --version` as coming *ahead of every other branch* (`src/main.rs:run`),
  which stops being a description of that block the moment `--help` and `--licenses` join it.
  Check 6 diffs stream bytes and is silent about that sentence — it was reached by reading,
  which is why it is written down here rather than left for the gate to miss. At its cap, so
  it meets the same raise-or-cut rule and says which. **`README.md` is substantially rewritten in its install and licence sections**, which
  is the largest documentation change since Phase 5 and the one a stranger actually reads.
  `proto/NOTICE.md` gains one line recording that Oko is published and under what expression;
  its provenance table is untouched. **The `CLAUDE.md` observable line is re-read and the
  expected answer is "no change"** — nothing here alters what a human sees. ~~Raises **OQ-13**
  and **OQ-14** and resolves neither;~~ check 6 produces OQ-14's evidence and check 1 produces
  the fact OQ-13 turns on, which is that the binaries are the whole product a stranger
  installs. **CORRECTED 2026-09-02 (this phase's own close-out): OQ-14 is resolved here, not
  merely raised.** The sentence above was written expecting check 6's evidence to inform a
  later decision; the evidence turned out to be decisive on arrival — byte-identical
  captures, header included — so the phase bumped to `0.2.0` and closed the question. **OQ-13
  is raised and not settled**, as written: nothing in this gate bears on whether the lib is
  published with a public surface. Commit plan: **one push to `feat/publish-crate`** — the spec and review-record
  commit first, then the manifest and rename, then the flags, then the documentation, and the
  PR leaves draft when the round converges. **`cargo publish` is not in this plan**: the
  branch ends at a verified tarball and a human presses the button, because the one thing
  crates.io does not offer is a second first release.

### Phase 8 — the binaries are the product: no library surface in the published crate

*Produces the observable: **no**, and the argument is Phase 7's with one word changed. That
phase produced a second population; this one changes what that population **receives**, and
for the person who types `cargo install oko-iterm2 && oko` the observable is identical by
construction — same dashboard, same stream, same three commands, byte for byte. Who it is
for is the person who would have written `oko-iterm2 = "0.2"` in a `[dependencies]` block:
they now cannot, which is the product of the phase. **The risk that makes this worth stating
is the inverse of Phase 7's**: that phase was metadata and could be done thoroughly with the
install left broken; this one is a build restructure that can leave every check green while
still shipping the surface it exists to remove. So the gate is built out of `cargo doc` and
the packaged manifest rather than out of `cargo build`.*

- **Scope.**
  - **`src/lib.rs` is deleted, and `[lib]` leaves `Cargo.toml`.** Phase 7 added
    `[lib] name = "oko"` for one reason — the package rename would otherwise have made it
    `oko_iterm2` and broken eleven imports (§2.15) — and that reason dies with the target.
    **The `[[bin]]` entries stay exactly as they are**: they name the binaries, which is a
    different job and the one Phase 7 was actually right about.
  - **Each binary declares the shared modules at its own root**, replacing `use oko::…`:
    `src/main.rs`, `src/bin/oko-hook.rs`, `src/bin/oko-probe.rs`. The two module trees
    (`src/iterm/`, `src/status.rs`) **do not move** — they are reached by `#[path]`, which is
    what keeps this a build change rather than a code change. `src/iterm/watch.rs`'s
    `crate::status` paths resolve unchanged inside a binary crate that declares both, and
    that is the property to confirm first rather than discover.
  - **Eleven `use oko::…` sites across five files** become plain `use crate::…` or `use
    iterm::…`: `src/main.rs`, `src/ui.rs`, `src/follow.rs`, `src/bin/oko-hook.rs`,
    `src/bin/oko-probe.rs`. Two further mentions are doc comments (the same eleven-real,
    thirteen-total split §2.15 measured for the rename).
  - **The 17 lib tests do not stay put, and this is the consequence to decide rather than
    meet.** `src/status.rs` and `src/iterm/watch.rs` carry `#[cfg(test)] mod tests`, and a
    module included by three binaries has its tests compiled and run **three times** — 17
    becomes 51 executions across three test binaries. That is noise, not failure, and the
    phase either accepts it in writing or gates the modules so one binary owns them. **It is
    named here because a plan that discovers it at `cargo test` will reach for the wrong
    fix**, which is deleting tests.
  - **`README.md`'s "How this repo is organised"** — nothing there promises a library today,
    which is worth confirming rather than assuming, and the licence paragraph's *"taking a
    library dependency on this crate takes the GPL-2.0 obligation with it"* becomes a
    sentence about something no longer possible.
  - **Not in scope, each a decision rather than an omission:** no move of `src/iterm/` or
    `src/status.rs` on disk — a reorganisation would bury this diff in renames and is the
    thing that makes a restructure unreviewable. No change to what any binary does. No
    `cargo publish`, for Phase 7's reason. **No version bump argued here**: the crate is
    unpublished, so `0.2.0` has never meant anything to anyone and this changes no string a
    consumer has parsed — which is the opposite of OQ-14's situation and is why that
    question does not reopen.
- **Exit gate.** Every check is headless; none needs iTerm2 or the API except 5, which is the
  one that proves nothing regressed. **Checks 1 and 2 are the phase**, and both are about the
  packaged artifact rather than the working tree.
  1. **The package has no lib target.** `cargo metadata` reports **zero** targets of kind
     `lib` and exactly three of kind `bin` (`oko`, `oko-hook`, `oko-probe`). Then the
     packaged manifest: `cargo package` and read `target/package/oko-iterm2-<v>/Cargo.toml` —
     **no `[lib]` section**. Phase 7's check 8 asserted one lib named `oko`; this is that
     assertion inverted, and it is the cheap form of the whole phase.
  2. **Nothing can depend on it.** In a scratch directory outside this repo, a crate whose
     `Cargo.toml` says `oko-iterm2 = { path = "…/target/package/oko-iterm2-<v>" }` and whose
     `src/main.rs` says `use oko_iterm2::…;` (and `use oko::…;`) **must fail to build**, with
     the error naming the missing crate rather than a missing item. **Run this before the
     change as well, and record that it *succeeded* then** — a check that only ever fails
     proves nothing about what it is measuring, and Phase 7's check 2 is the precedent.
  3. **`cargo test` accounts for all 17.** Whichever way the previous bullet's duplication
     lands, name the number the suite reports and why: 51 across three binaries, or 17 with
     the modules gated to one. **A total that is neither is a test that stopped running.**
  4. **`cargo build --release`, `cargo clippy -- -D warnings` and `cargo package` are clean**,
     and `spec-lint --strict` passes. `#[path]` inclusion is the kind of change that compiles
     while leaving a module unreferenced, so clippy's dead-code warnings are load-bearing here
     rather than decoration.
  5. **Nothing regressed, and the stream proves it cheapest.** `oko --follow` against a real
     window is byte-identical to Phase 7's, header included — same version, same `schema`, no
     row field moved. Then `oko --help`, `--licenses`, `--version` and a typo behave as
     Phase 7's check 2 recorded, and `oko-probe` still enumerates. **The dashboard is not
     re-gated by keystroke**: no code in `src/ui.rs` changes except its `use` lines, and
     check 5's stream comparison exercises the same `Watcher`.
  6. **`cargo +1.88.0 check --bins` still succeeds.** The floor was derived from the locked
     graph and nothing here touches dependencies, so this is a regression check rather than a
     derivation — but three binaries now compile code one used to, and `#[path]` is not what
     an MSRV is usually tested against.
- **Close-out.** **Reconciliation.** `rules/iterm-api.md` and `rules/claude-status.md` both
  declare `sources` under `src/iterm/` and `src/status.rs`, which do not move — **but their
  citations are `file:symbol` and stay resolvable by construction**, so the expected answer is
  "no change" and check 4's linter is the backstop that makes that expectation falsifiable.
  `rules/session-commands.md` and `rules/follow-stream.md` both name `src/main.rs` symbols;
  same reasoning. **`rules/dashboard-ui.md` is the one to read rather than assume** — it
  describes three threads and the branches ahead of them, which is where a binary's root
  module declarations now sit. `README.md`'s licence paragraph loses the sentence about a
  library dependency, which is the only user-facing change in the phase. `proto/NOTICE.md`
  says the same thing and changes with it. **The `CLAUDE.md` observable line is re-read and
  the expected answer is "no change"**. Resolves **OQ-13** — recorded in §2.16 and in §3 —
  and raises none. Commit plan: **one push to `feat/no-lib-surface`** — the spec and
  review-record commit first, then the restructure, then the documentation; the PR leaves
  draft when the round converges. `cargo publish` is not in this plan either, and is now
  unblocked by it.
