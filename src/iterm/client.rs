//! One connection to the iTerm2 API: the handshake, requests, and the frames that arrive
//! without being asked for.
//!
//! Phase 1 proved this protocol. What it did not prove is the *shape* Phase 2 needs:
//! responses and notifications share one frame stream, so a client that waits for a
//! response by discarding everything else eats notifications — silently, and only while a
//! request happens to be in flight. [`Client::call`] queues them instead, and
//! [`Client::next_notification`] hands them back before it reads the socket again.

use std::collections::{HashMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use prost::Message as _;
use tungstenite::{Message as WsMessage, WebSocket, http};

use super::api::{
    self, ActivateRequest, ClientOriginatedMessage, ListSessionsRequest, ListSessionsResponse,
    Notification, NotificationRequest, NotificationType, ServerOriginatedMessage,
    VariableMonitorRequest, VariableRequest, VariableScope,
    client_originated_message::Submessage as Req, server_originated_message::Submessage as Resp,
};

/// How long a request waits for its response before the connection is called dead. Long,
/// because the only thing that should ever take this long is iTerm2 being wedged.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Client {
    stream: WebSocket<UnixStream>,
    next_id: i64,
    /// Notifications that arrived while a request was in flight. Never dropped: this is
    /// the frame the dashboard is built out of.
    pending: VecDeque<Notification>,
    protocol_version: Option<String>,
}

/// One frame off the socket, or nothing because the read timed out.
enum Frame {
    Message(Box<ServerOriginatedMessage>),
    Idle,
}

impl Client {
    /// Connects and authorizes. `advisory_name` is what iTerm2 shows in its API console
    /// and remembers in its permissions list, so each binary gets its own.
    pub fn connect(advisory_name: &str) -> Result<Self> {
        let path = socket_path();
        if !path.exists() {
            bail!(
                "no API socket at {}.\n\
                 The iTerm2 Python API is off. Turn it on in iTerm2 → Settings → General →\n\
                 Magic → Enable Python API; the socket appears immediately, no restart needed.",
                path.display()
            );
        }

        // The official client sends `python <version>`. Whether iTerm2 parses that shape or
        // merely logs it is not documented, so try our own name first and fall back.
        // A cookie is spent by the attempt that uses it, so each attempt asks for its own —
        // otherwise a second try fails on a stale cookie and blames the version header.
        let mut last_err = None;
        for library_version in ["oko 1.0", "python 2.10"] {
            let (cookie, key) = request_cookie_and_key(advisory_name)?;
            match Self::handshake(&path, &cookie, &key, library_version, advisory_name) {
                Ok((stream, protocol_version)) => {
                    return Ok(Client {
                        stream,
                        next_id: 1,
                        pending: VecDeque::new(),
                        protocol_version,
                    });
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.expect("the loop ran at least once"))
    }

    fn handshake(
        path: &PathBuf,
        cookie: &str,
        key: &str,
        library_version: &str,
        advisory_name: &str,
    ) -> Result<(WebSocket<UnixStream>, Option<String>)> {
        let socket =
            UnixStream::connect(path).with_context(|| format!("connecting to {}", path.display()))?;
        socket.set_read_timeout(Some(CALL_TIMEOUT))?;

        // Handed a pre-built request, tungstenite generates none of the mandatory handshake
        // headers and rejects the request if any of the five is missing — it only synthesizes
        // them for the `&str`-URI form, which cannot carry the x-iterm2-* headers below.
        let request = http::Request::builder()
            .uri("ws://localhost/")
            .header("Host", "localhost")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .header("Origin", "ws://localhost/")
            .header("Sec-WebSocket-Protocol", "api.iterm2.com")
            .header("x-iterm2-library-version", library_version)
            .header("x-iterm2-cookie", cookie)
            .header("x-iterm2-key", key)
            .header("x-iterm2-advisory-name", advisory_name)
            .body(())?;

        let (stream, response) = tungstenite::client(request, socket).map_err(|e| match e {
            tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) => {
                let body =
                    r.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default().to_string();
                anyhow!("iTerm2 refused the handshake: HTTP {} {body}", r.status())
            }
            other => anyhow!("handshake failed: {other}"),
        })?;

        let protocol_version = response
            .headers()
            .get("X-iTerm2-Protocol-Version")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok((stream, protocol_version))
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    /// How long [`next_notification`](Self::next_notification) waits before reporting that
    /// nothing arrived. Sets the pace at which the watcher notices a queued command.
    pub fn set_read_timeout(&mut self, timeout: Duration) -> Result<()> {
        self.stream.get_mut().set_read_timeout(Some(timeout))?;
        Ok(())
    }

    /// Sends one request and returns the response carrying the same id.
    ///
    /// Notifications arriving in the meantime are **queued, not discarded** — a variable
    /// change that lands while an activate is in flight is exactly the update the table
    /// exists to show.
    pub fn call(&mut self, submessage: Req) -> Result<Resp> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = ClientOriginatedMessage { id: Some(id), submessage: Some(submessage) };
        self.stream.send(WsMessage::binary(msg.encode_to_vec()))?;

        let deadline = Instant::now() + CALL_TIMEOUT;
        loop {
            match self.read_frame()? {
                Frame::Idle => {
                    if Instant::now() >= deadline {
                        bail!("iTerm2 did not answer request {id} within {CALL_TIMEOUT:?}");
                    }
                }
                Frame::Message(response) => {
                    if response.id != Some(id) {
                        if let Some(Resp::Notification(n)) = response.submessage {
                            self.pending.push_back(n);
                        }
                        continue;
                    }
                    return match response.submessage {
                        Some(Resp::Error(e)) => bail!("iTerm2 rejected the request: {e}"),
                        Some(other) => Ok(other),
                        None => bail!("iTerm2 returned an empty response"),
                    };
                }
            }
        }
    }

    /// The next notification, waiting at most one read timeout for it.
    ///
    /// `Ok(None)` means nothing arrived, which is the ordinary case: it is what gives the
    /// watcher's loop a chance to look at its command channel.
    pub fn next_notification(&mut self) -> Result<Option<Notification>> {
        if let Some(n) = self.pending.pop_front() {
            return Ok(Some(n));
        }
        match self.read_frame()? {
            Frame::Idle => Ok(None),
            Frame::Message(msg) => Ok(match msg.submessage {
                Some(Resp::Notification(n)) => Some(n),
                // A response nobody is waiting for: the request that wanted it timed out.
                _ => None,
            }),
        }
    }

    /// One frame, or `Idle` when the read timed out.
    ///
    /// tungstenite keeps its partial-frame state across a `WouldBlock`, so a read timeout
    /// is resumable rather than corrupting: the next call picks the frame up where this
    /// one left it.
    fn read_frame(&mut self) -> Result<Frame> {
        loop {
            return match self.stream.read() {
                Ok(WsMessage::Binary(b)) => ServerOriginatedMessage::decode(&b[..])
                    .map(|m| Frame::Message(Box::new(m)))
                    .context("decoding a ServerOriginatedMessage"),
                Ok(WsMessage::Close(frame)) => bail!("iTerm2 closed the connection: {frame:?}"),
                // Ping/Pong are answered by tungstenite; Text is not something this API sends.
                Ok(_) => continue,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    Ok(Frame::Idle)
                }
                Err(e) => Err(e.into()),
            };
        }
    }

    pub fn subscribe(
        &mut self,
        notification_type: NotificationType,
        variable_monitor: Option<VariableMonitorRequest>,
    ) -> Result<()> {
        let resp = self.call(Req::NotificationRequest(NotificationRequest {
            session: None,
            subscribe: Some(true),
            notification_type: Some(notification_type as i32),
            arguments: variable_monitor
                .map(api::notification_request::Arguments::VariableMonitorRequest),
        }))?;
        let Resp::NotificationResponse(r) = resp else {
            bail!("expected a notification response, got {resp:?}");
        };
        match api::notification_response::Status::try_from(r.status.unwrap_or_default()) {
            Ok(api::notification_response::Status::Ok) => Ok(()),
            Ok(status) => bail!("subscribing to {notification_type:?} failed: {status:?}"),
            Err(_) => bail!("subscribe returned an unknown status: {:?}", r.status),
        }
    }

    /// Subscribes to changes of one session-scope variable. Both halves matter: a
    /// subscription covers one session *and* one variable, so a session that appears later
    /// is covered only by subscribing it on arrival.
    pub fn watch_variable(&mut self, session_id: &str, name: &str) -> Result<()> {
        self.subscribe(
            NotificationType::NotifyOnVariableChange,
            Some(VariableMonitorRequest {
                name: Some(name.to_string()),
                scope: Some(VariableScope::Session as i32),
                identifier: Some(session_id.to_string()),
            }),
        )
    }

    pub fn list_sessions(&mut self) -> Result<ListSessionsResponse> {
        let resp = self.call(Req::ListSessionsRequest(ListSessionsRequest {}))?;
        let Resp::ListSessionsResponse(list) = resp else {
            bail!("expected a list-sessions response, got {resp:?}");
        };
        Ok(list)
    }

    /// Session-scope variables, by name.
    ///
    /// `ListSessions` alone is not enough: its `SessionSummary` carries only an identifier,
    /// a frame, a grid size and a title — neither the directory nor the job name a row is
    /// made of. Those take one `VariableRequest` per session.
    pub fn variables(
        &mut self,
        session_id: &str,
        names: &[&str],
    ) -> Result<HashMap<String, String>> {
        let resp = self.call(Req::VariableRequest(VariableRequest {
            scope: Some(api::variable_request::Scope::SessionId(session_id.to_string())),
            set: vec![],
            get: names.iter().map(|s| (*s).to_string()).collect(),
        }))?;
        let Resp::VariableResponse(r) = resp else {
            bail!("expected a variable response, got {resp:?}");
        };
        if let Ok(status) = api::variable_response::Status::try_from(r.status.unwrap_or_default())
            && status != api::variable_response::Status::Ok
        {
            bail!("reading variables of {session_id} failed: {status:?}");
        }

        // Values are 1:1 with the `get` list and JSON-encoded, with `null` for unset.
        let mut vars = HashMap::new();
        for (name, raw) in names.iter().zip(&r.values) {
            if let Some(value) = decode_json_value(raw) {
                vars.insert((*name).to_string(), value);
            }
        }
        Ok(vars)
    }

    /// Sets one session-scope variable.
    ///
    /// **The name must begin with `user.`** — iTerm2 answers `INVALID_NAME` otherwise, which
    /// is the whole reason Oko's own keys are spelled that way. The value is JSON-encoded,
    /// the same encoding [`variables`](Self::variables) decodes on the way back, and `null`
    /// unsets it.
    pub fn set_variable(&mut self, session_id: &str, name: &str, value: &str) -> Result<()> {
        let resp = self.call(Req::VariableRequest(VariableRequest {
            scope: Some(api::variable_request::Scope::SessionId(session_id.to_string())),
            set: vec![api::variable_request::Set {
                name: Some(name.to_string()),
                value: Some(value.to_string()),
            }],
            get: vec![],
        }))?;
        let Resp::VariableResponse(r) = resp else {
            bail!("expected a variable response, got {resp:?}");
        };
        match api::variable_response::Status::try_from(r.status.unwrap_or_default()) {
            Ok(api::variable_response::Status::Ok) => Ok(()),
            Ok(status) => bail!("setting {name} on {session_id} failed: {status:?}"),
            Err(_) => bail!("setting {name} returned an unknown status: {:?}", r.status),
        }
    }

    pub fn activate(&mut self, session_id: &str) -> Result<()> {
        let resp = self.call(Req::ActivateRequest(ActivateRequest {
            identifier: Some(api::activate_request::Identifier::SessionId(session_id.to_string())),
            order_window_front: Some(true),
            select_tab: Some(true),
            select_session: Some(true),
            activate_app: Some(api::activate_request::App {
                raise_all_windows: Some(false),
                ignoring_other_apps: Some(true),
            }),
        }))?;
        let Resp::ActivateResponse(r) = resp else {
            bail!("expected an activate response, got {resp:?}");
        };
        match api::activate_response::Status::try_from(r.status.unwrap_or_default()) {
            Ok(api::activate_response::Status::Ok) => Ok(()),
            Ok(status) => bail!("activate failed: {status:?}"),
            Err(_) => bail!("activate returned an unknown status: {:?}", r.status),
        }
    }
}

/// A variable's value as the API encodes it: JSON, `null` when unset. A string comes back
/// unquoted and unescaped (`"\/tmp"` is `/tmp`); anything else keeps its JSON spelling,
/// which is only ever a display value here.
pub(crate) fn decode_json_value(raw: &str) -> Option<String> {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Null) => None,
        Ok(serde_json::Value::String(s)) => Some(s),
        Ok(other) => Some(other.to_string()),
        Err(_) => Some(raw.to_string()),
    }
}

pub fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let suite = std::env::var("IT2_SUITE").unwrap_or_else(|_| "iTerm2".to_string());
    PathBuf::from(home).join("Library/Application Support").join(suite).join("private/socket")
}

/// iTerm2 hands out a cookie and key over AppleScript, space-separated. No macOS Automation
/// grant is involved for a client running inside iTerm2 — see `rules/iterm-api.md`.
fn request_cookie_and_key(advisory_name: &str) -> Result<(String, String)> {
    let script = format!(
        r#"tell application "iTerm2" to request cookie and key for app named "{advisory_name}""#
    );
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("running osascript to request an API cookie")?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "iTerm2 would not issue an API cookie: {}\n\
             See rules/iterm-api.md for how the authorization works and how to reset a grant.",
            err.trim()
        );
    }

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    text.split_once(' ')
        .map(|(c, k)| (c.to_string(), k.to_string()))
        .ok_or_else(|| anyhow!("expected `<cookie> <key>` from osascript, got {text:?}"))
}
