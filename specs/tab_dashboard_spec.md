---
id: oko-001
title: tab-dashboard
note: >
  The iTerm2 dashboard tab — live per-tab directory, process and Claude Code status for
  every tab in the window, with Enter to jump to the selected one.
status: accepted
last_updated: 2026-08-16

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
  `src/bin/probe.rs:var_spike` against iTerm2 3.6.11, writing to a session the probe does not
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
  **re-pointing its `sources` at `src/iterm/`**, and saying whether `src/bin/probe.rs`
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
    `src/bin/probe.rs:var_spike`, which the round added and which stays as a diagnostic;
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
