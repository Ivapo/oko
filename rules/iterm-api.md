---
title: iterm-api
sources:
  - src/iterm/client.rs
  - src/iterm/watch.rs
  - proto/api.proto
covers: >
  how Oko reaches the iTerm2 scripting API — the endpoint, how a human enables it, how a
  client authorizes and how a grant is reset, the transport, the session join key, and the
  variables it reads, writes and watches, the operations and the subscriptions
max_lines: 115
generated: 2026-08-16
---

# iTerm2 API

A **WebSocket server inside iTerm2** carrying protobuf over a Unix domain socket at
`~/Library/Application Support/${IT2_SUITE:-iTerm2}/private/socket`
(`src/iterm/client.rs:socket_path`) — measured against iTerm2 3.6.11 on 2026-08-14, protocol
version 1.11, reported back in `X-iTerm2-Protocol-Version`. The socket exists only while
the API is enabled, so its absence is the "API is off" signal rather than a wrong path.

## Setup, and undoing it

Enabling is a human step, once per machine: iTerm2 → Settings (⌘,) → General → Magic →
**Enable Python API**, which raises iTerm2's own *Enable Python API?* confirmation. That
dialog is **the only prompt a human sees**, and it is about the feature, not about any
particular client. Effective at once, no restart; the socket appears and
`defaults read com.googlecode.iterm2` gains `EnableAPIServer = 1`. A client then gets a
cookie and key from AppleScript, printed space-separated
(`src/iterm/client.rs:request_cookie_and_key`):

    osascript -e 'tell application "iTerm2" to request cookie and key for app named "oko"'

**No Automation grant is involved when the client runs inside iTerm2**, which Oko always
does — it is a tab of the window it watches. macOS attributes the `osascript` call to
iTerm2 as the responsible process, and an app scripting itself needs no grant: `tccd`
logged **zero** `kTCCServiceAppleEvents` requests across every connection made on
2026-08-14, so no Automation entry appears and none should be looked for. A client started
*outside* iTerm2 is attributed to whatever launched it and would need the grant; that is
what `tccutil reset AppleEvents com.googlecode.iterm2` and System Settings → Privacy &
Security → Automation undo. **A cookie is spent by the connection that uses it** — a retry
needs a fresh one, or it fails on a stale cookie and looks unrelated.

## Transport

Blocking `tungstenite` over `std::os::unix::net::UnixStream` — no async runtime, no Python
sidecar (`src/iterm/client.rs:handshake`). Handed a pre-built `http::Request`, tungstenite
generates **none** of the mandatory handshake headers (Host, Connection, Upgrade,
Sec-WebSocket-Version, Sec-WebSocket-Key) and errors if one is absent; it synthesizes them
only for the `&str`-URI form, which cannot carry `Origin: ws://localhost/`,
`Sec-WebSocket-Protocol: api.iterm2.com`, `x-iterm2-cookie`, `x-iterm2-key`,
`x-iterm2-advisory-name` or `x-iterm2-library-version` — where **`oko 1.0` is accepted**,
so it need not imitate the official `python <version>`. Each frame is one binary
`ClientOriginatedMessage` or `ServerOriginatedMessage`; responses echo the request's `id`,
notifications carry none, and **one stream carries both** — so a client waiting on a
response queues the notifications arriving meanwhile rather than discarding them
(`src/iterm/client.rs:call`), or it loses an update exactly while a request is in flight.
Serving that socket from one thread needs a read timeout, which is safe because tungstenite
keeps partial-frame state across a `WouldBlock` (`src/iterm/client.rs:read_frame`).
`proto/api.proto` is vendored verbatim from `gnachman/iTerm2`, commit `f4ca0004`, sha256
`6f1a4e75…`, fetched 2026-08-14 — self-contained, so `protox` and `prost-build` compile it
in `build.rs` with no `protoc`.

## The join key

**A session's `id` variable equals the UUID after the colon in `TERM_SESSION_ID`**, and
that is what Oko joins on (`src/iterm/watch.rs:resolve_own_session`) — the same string
`ListSessions` returns as `unique_identifier`. Both fallbacks join too:
`tty` equals `ttyname()` of fds 0–2 — *not* of a freshly opened `/dev/tty`, a cloning
device that answers `/dev/tty` (`src/iterm/watch.rs:own_tty`) — and `termid` is
`wNtMpK.<uuid>`, dot-separated where `TERM_SESSION_ID` uses a colon, so matching a
position compares only the prefix. **`t` is a monotonic id, not a tab position**: one
window reported `t0 t1 t2 t3 t4 t5 t8` across seven tabs. No tab index exists anywhere in
the API — `Tab` carries only `tab_id`, and tab scope has no index variable — though
`Window` does carry `number`. What stands in for one is the 1-based position in
`windows[].tabs[]`, and **that ordering is the tab bar's own**: confirmed 2026-08-15 by
dragging a tab to a new position and watching the column follow, there and back.

## Variables, operations, subscriptions

`ListSessions` returns `SessionSummary`, carrying **no directory and no job name** — only
an identifier, frame, grid size and title. Those take one `VariableRequest` per session
(`src/iterm/client.rs:variables`), answered JSON-encoded one per requested name, solidus
escaped (`"\/tmp"`). **`jobName` is truncated to 16 bytes** (`MAXCOMLEN`):
`rust-analyzer-proc-macro-srv` reports as `rust-analyzer-pr`. A tab is a tree of sessions,
so enumeration recurses through `SplitTreeNode` (`src/iterm/watch.rs:flatten`); buried
sessions belong to no window and are excluded from **rows** — but they are alive, so
anything asking "does this session still exist" must add `buried_sessions` back
(`src/iterm/watch.rs:sweep_status`). `ActivateRequest` with
`order_window_front`, `select_tab` and `select_session` focuses a session and its tab and
raises its window.

**Writing a variable works on a session this process does not occupy**, measured 2026-08-15
by `src/bin/probe.rs:var_spike` against 3.6.11 — set, read back, and a
`NOTIFY_ON_VARIABLE_CHANGE` on the same key delivering the new value and the session id.
`VariableRequest.set` carries `{name, value}` and **the name must begin with `user.`**, or
iTerm2 answers `INVALID_NAME`; that is why Oko's key is `user.okoName`
(`src/iterm/client.rs:set_variable`). The value is **raw JSON text and the caller encodes
it** — `serde_json::Value::String(..).to_string()`, not Rust's `Debug`, which is not a JSON
escaper. **JSON `null` unsets**, and the variable then reads back *absent* rather than as an
empty string: `src/iterm/client.rs:decode_json_value` maps `""` to `Some("")`, so the empty
string would be a value rather than an absence. A `user.` key can be watched exactly like a
built-in one, which is what lets two Oko instances see one name with no protocol between
them.
Subscriptions deliver `NOTIFY_ON_VARIABLE_CHANGE` (per session *and* variable),
`NOTIFY_ON_NEW_SESSION`, `NOTIFY_ON_TERMINATE_SESSION`, `NOTIFY_ON_LAYOUT_CHANGE`.
Oko subscribes `path`, `jobName` and `user.okoName` per session (`ROW_VARS`), plus layout
and new-session. A
session created after subscribing is not covered by the per-session ones, so it must be
subscribed when its notification arrives. **The two variables have different latencies.**
`jobName` is poll-driven: a 5.000 s `sleep` produced its two notifications 5.602 s apart.
`path` is pushed on the `cd` itself — observed immediate to the eye, watching a pane and
the table side by side (2026-08-15).

Payloads differ in a way that decides how a client is built. `NewSessionNotification` and
`TerminateSessionNotification` carry a `session_id` and nothing else — no window, no tab —
so neither can place a session. `LayoutChangedNotification` carries a whole
`ListSessionsResponse`, delivering the new shape of every window inline, and measured
2026-08-15 it fires for a pane split, a tab drag and a tab close alike — a drag creating no
session, terminating none and changing no variable, so it is the only event that carries
one. It does **not** fire for a tab *opening*: measured 2026-08-16, that sends
`NewSessionNotification` and nothing else, and the earlier reading that layout change made
the session notifications redundant was drawn from events that all happened to be closes.
A client that wants new tabs subscribes to both and calls `ListSessions` itself when the
session notification arrives, since that payload cannot place anything on its own.
