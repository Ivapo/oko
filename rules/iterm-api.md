---
title: iterm-api
sources:
  - src/bin/probe.rs
  - proto/api.proto
covers: >
  how Oko reaches the iTerm2 scripting API — the endpoint, how a human enables it, how a
  client authorizes and how a grant is reset, the transport, the session join key, and the
  variables, operations and subscriptions Oko uses
max_lines: 85
generated: 2026-08-14
---

# iTerm2 API

A **WebSocket server inside iTerm2** carrying protobuf over a Unix domain socket at
`~/Library/Application Support/${IT2_SUITE:-iTerm2}/private/socket`
(`src/bin/probe.rs:socket_path`) — measured against iTerm2 3.6.11 on 2026-08-14, protocol
version 1.11, reported back in `X-iTerm2-Protocol-Version`. The socket exists only while
the API is enabled, so its absence is the "API is off" signal rather than a wrong path.

## Setup, and undoing it

Enabling is a human step, once per machine: iTerm2 → Settings (⌘,) → General → Magic →
**Enable Python API**, which raises iTerm2's own *Enable Python API?* confirmation. That
dialog is **the only prompt a human sees**, and it is about the feature, not about any
particular client. Effective at once, no restart; the socket appears and
`defaults read com.googlecode.iterm2` gains `EnableAPIServer = 1`. A client then gets a
cookie and key from AppleScript, printed space-separated
(`src/bin/probe.rs:request_cookie_and_key`):

    osascript -e 'tell application "iTerm2" to request cookie and key for app named "oko"'

**No Automation grant is involved when the client runs inside iTerm2**, which Oko always
does — it is a tab of the window it watches. macOS attributes the `osascript` call to
iTerm2 as the responsible process, and an app scripting itself needs no grant: `tccd`
logged **zero** `kTCCServiceAppleEvents` requests across every connection made on
2026-08-14. Expect no Automation entry to appear, and do not go looking for one. A client
started *outside* iTerm2 is attributed to whatever launched it and would need the grant;
that is what `tccutil reset AppleEvents com.googlecode.iterm2` and System Settings →
Privacy & Security → Automation undo. **A cookie is spent by the connection that uses it**
— a retry needs a fresh one, or it fails on a stale cookie and looks unrelated. Turning
the API off removes the socket and drops every connection.

## Transport

Blocking `tungstenite` over `std::os::unix::net::UnixStream` — no async runtime, no Python
sidecar (`src/bin/probe.rs:handshake`). Handed a pre-built `http::Request`, tungstenite
generates **none** of the mandatory handshake headers and errors if any of Host /
Connection / Upgrade / Sec-WebSocket-Version / Sec-WebSocket-Key is absent; it synthesizes
them only for the `&str`-URI form, which cannot carry the rest: `Origin: ws://localhost/`,
`Sec-WebSocket-Protocol: api.iterm2.com`, `x-iterm2-cookie`, `x-iterm2-key`,
`x-iterm2-advisory-name`, and `x-iterm2-library-version` — where **`oko 1.0` is
accepted**, so it need not imitate the official `python <version>`. Each frame is one
binary `ClientOriginatedMessage` or `ServerOriginatedMessage`; responses echo the
request's `id`, notifications carry none.
`proto/api.proto` is vendored verbatim from `gnachman/iTerm2`, commit `f4ca0004`, sha256
`6f1a4e75…`, fetched 2026-08-14 — self-contained, so `protox` and `prost-build` compile it
in `build.rs` with no `protoc`.

## The join key

**A session's `id` variable equals the UUID after the colon in `TERM_SESSION_ID`**, and
that is what Oko joins on (`src/bin/probe.rs:report_identity`). It is also the same string
`ListSessions` returns as `unique_identifier`. Both fallbacks join too:
`tty` equals `ttyname()` of fds 0–2 — *not* of a freshly opened `/dev/tty`, a cloning
device that answers `/dev/tty` (`src/bin/probe.rs:own_tty`) — and `termid` is
`wNtMpK.<uuid>`, dot-separated where `TERM_SESSION_ID` uses a colon, so matching a
position compares only the prefix. **`t` is a monotonic id, not a tab position**: one
window reported `t0 t1 t2 t3 t4 t5 t8` across seven tabs. No tab index exists anywhere in
the API — `Tab` carries only `tab_id`, and tab scope has no index variable — though
`Window` does carry `number`.

## Variables, operations, subscriptions

`ListSessions` returns `SessionSummary`, carrying **no directory and no job name** — only
an identifier, frame, grid size and title. Those take one `VariableRequest` per session
(`src/bin/probe.rs:WANTED_VARS`), answered JSON-encoded one per requested name, solidus
escaped (`"\/tmp"`). **`jobName` is truncated to 16 bytes** (`MAXCOMLEN`):
`rust-analyzer-proc-macro-srv` reports as `rust-analyzer-pr`. A tab is a tree of sessions,
so enumeration recurses through `SplitTreeNode` (`src/bin/probe.rs:flatten`); buried
sessions belong to no window and are excluded. `ActivateRequest` with
`order_window_front`, `select_tab` and `select_session` focuses a session and its tab and
raises its window.
Subscriptions deliver `NOTIFY_ON_VARIABLE_CHANGE` (per session *and* variable),
`NOTIFY_ON_NEW_SESSION`, `NOTIFY_ON_TERMINATE_SESSION`, `NOTIFY_ON_LAYOUT_CHANGE`. A
session created after subscribing is not covered by the per-session ones, so it must be
subscribed when its notification arrives. `jobName` is poll-driven: a 5.000 s `sleep`
produced its two notifications 5.602 s apart.

Payloads differ in a way that decides how a client is built. `NewSessionNotification` and
`TerminateSessionNotification` carry a `session_id` and nothing else — no window, no tab —
so neither can place a session on its own. `LayoutChangedNotification` carries a whole
`ListSessionsResponse`, so it delivers the new shape of every window inline; it is also
the **only** event that fires when a tab is reordered, which creates no session,
terminates none and changes no session variable.
