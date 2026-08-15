//! Phase 1 of `specs/tab_dashboard_spec.md` — the transport spike.
//!
//! Throwaway-grade on purpose. It answers OQ-1 (how a Rust binary reaches the iTerm2 API),
//! establishes the join key §2.4 rests on, and observes what OQ-3 asked about variable-change
//! subscriptions. Phase 2 rewrites all of this as `src/iterm/`.
//!
//!     probe                      identity, then the sessions of this window
//!     probe activate <session>   focus that session, its tab, and its window
//!     probe watch                print variable-change notifications as they arrive
//!
//! Setup — enabling the API, authorizing this binary — is in `rules/iterm-api.md`.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use prost::Message as _;
use tungstenite::{Message as WsMessage, WebSocket, http};

// Generated from the vendored schema; its shape is iTerm2's business, not ours.
#[allow(clippy::all, clippy::pedantic)]
mod api {
    // prost writes one file per proto package; ours is `iterm2`.
    include!(concat!(env!("OUT_DIR"), "/iterm2.rs"));
}

use api::{
    ActivateRequest, ClientOriginatedMessage, ListSessionsRequest, ListSessionsResponse,
    NotificationRequest, NotificationType, ServerOriginatedMessage, SessionSummary,
    SplitTreeNode, VariableMonitorRequest, VariableRequest, VariableScope,
    client_originated_message::Submessage as Req, server_originated_message::Submessage as Resp,
};

/// The name iTerm2 shows in its authorization prompt and in its API console. Held as a
/// constant because it is interpolated into an AppleScript source string below.
const ADVISORY_NAME: &str = "oko-probe";

/// The session-scope variables the spike needs: three identity candidates (§2.1) and the
/// two values a plain row is made of (§2.2).
const WANTED_VARS: [&str; 5] = ["id", "tty", "termid", "path", "jobName"];

fn main() {
    if let Err(e) = run() {
        eprintln!("probe: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => enumerate(),
        Some("activate") => {
            let session = args.get(1).ok_or_else(|| anyhow!("usage: probe activate <session-id>"))?;
            activate(session)
        }
        Some("watch") => watch(),
        Some(other) => bail!("unknown command {other:?}; expected `activate <session-id>` or `watch`"),
    }
}

// ---------------------------------------------------------------------------------------
// Step 1 — identity, and step 2 — enumeration scoped by it
// ---------------------------------------------------------------------------------------

fn enumerate() -> Result<()> {
    let local = Local::read();
    let mut client = Client::connect()?;
    let sessions = client.sessions()?;

    let joined = report_identity(&local, &sessions)?;

    let window = sessions
        .iter()
        .find(|s| s.id == joined.session_id)
        .map(|s| s.window_id.clone())
        .expect("the joined session came out of this list");
    let mine: Vec<&Session> = sessions.iter().filter(|s| s.window_id == window).collect();

    println!();
    println!("── sessions in this window ──────────────────────────────────────────────");
    println!("{} session(s) in window {}:", mine.len(), window);
    for s in &mine {
        println!(
            "{:<38}  {:<10}  {}",
            s.id,
            s.var("jobName").unwrap_or("-"),
            s.var("path").unwrap_or("-")
        );
    }

    let elsewhere = sessions.len() - mine.len();
    if elsewhere > 0 {
        println!();
        println!("({elsewhere} further session(s) exist in other windows and are not listed)");
    }
    Ok(())
}

/// What the probe knows about its own pane without asking the API. Each of these is
/// searched for in the API's per-session values, so no candidate needs the answer in
/// order to ask the question.
struct Local {
    term_session_id: Option<String>,
    uuid: Option<String>,
    termid: Option<String>,
    tty: Option<String>,
}

impl Local {
    fn read() -> Self {
        let term_session_id = std::env::var("TERM_SESSION_ID")
            .or_else(|_| std::env::var("ITERM_SESSION_ID"))
            .ok();
        // Observed shape: `w0t2p0:F79BC39C-B1C1-47C3-9E9D-6820789978D9` — a window/tab/pane
        // triple, a colon, then a session UUID.
        let (termid, uuid) = match term_session_id.as_deref().and_then(|s| s.split_once(':')) {
            Some((prefix, uuid)) => (Some(prefix.to_string()), Some(uuid.to_string())),
            None => (None, None),
        };
        Local { term_session_id, uuid, termid, tty: own_tty() }
    }
}

/// The pane's tty device, as `/dev/ttysNNN`.
///
/// Asked about the standard descriptors, not about a freshly opened `/dev/tty`: that is a
/// cloning device with a device number of its own, so `ttyname` on it answers `/dev/tty` —
/// true, useless, and never equal to what the API reports. Any of the three will do, and
/// trying all three survives a redirected stdin or a piped stdout; when none is a terminal
/// the probe is not running in a pane and the candidate is simply unavailable.
fn own_tty() -> Option<String> {
    (0..=2).find_map(|fd| {
        // SAFETY: 0, 1 and 2 are valid descriptor numbers to interrogate whether or not
        // they are open. ttyname returns a pointer into thread-local storage, valid until
        // the next ttyname call on this thread, and null when fd is not a terminal.
        unsafe {
            let p = libc::ttyname(fd);
            (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
        }
    })
}

struct Joined {
    session_id: String,
}

/// Prints every candidate's local value beside what the API reports, so the two can be
/// compared by eye rather than by a match/no-match verdict, then names the one that joins.
fn report_identity(local: &Local, sessions: &[Session]) -> Result<Joined> {
    println!("── identity ─────────────────────────────────────────────────────────────");
    println!(
        "own TERM_SESSION_ID   {}",
        local.term_session_id.as_deref().unwrap_or("(unset — not running inside an iTerm2 pane)")
    );
    println!("own /dev/tty          {}", local.tty.as_deref().unwrap_or("(unavailable — no controlling terminal)"));
    println!();
    println!("what the API reports for every session it knows about:");
    println!("  {:<38}  {:<14}  {:<8}  jobName", "id", "tty", "termid");
    for s in sessions {
        println!(
            "  {:<38}  {:<14}  {:<8}  {}",
            s.var("id").unwrap_or("-"),
            s.var("tty").unwrap_or("-"),
            s.var("termid").unwrap_or("-"),
            s.var("jobName").unwrap_or("-")
        );
    }
    println!();

    // §2.1's ordered fallback: UUID, then tty device, then the wNtMpK prefix — the last
    // being a position, which expires the moment a tab is moved.
    //
    // Candidate 3 compares on a prefix rather than the whole value, and has to: the `termid`
    // variable is `wNtMpK` *and* the UUID joined by a dot, where `TERM_SESSION_ID` joins the
    // same two with a colon. Comparing the whole string would smuggle candidate 1's answer
    // into candidate 3 and stop testing the position at all.
    type Match<'a> = Box<dyn Fn(&str, &str) -> bool + 'a>;
    let whole: Match = Box::new(|local, api| local == api);
    let dotted_prefix: Match = Box::new(|local, api| api.starts_with(&format!("{local}.")));

    let candidates: [(&str, &str, &str, Option<&str>, &Match); 3] = [
        ("1", "session UUID", "id", local.uuid.as_deref(), &whole),
        ("2", "tty device", "tty", local.tty.as_deref(), &whole),
        ("3", "wNtMpK prefix", "termid", local.termid.as_deref(), &dotted_prefix),
    ];

    println!("  {:<2} {:<14} {:<8} {:<40} verdict", "#", "candidate", "variable", "local value");
    let mut winner: Option<(&str, &str, String)> = None;
    for (n, label, var, local_value, matches) in candidates {
        let Some(local_value) = local_value else {
            println!("  {n:<2} {label:<14} {var:<8} {:<40} unavailable", "-");
            continue;
        };
        let hits: Vec<&Session> = sessions
            .iter()
            .filter(|s| s.var(var).is_some_and(|api| matches(local_value, api)))
            .collect();
        let verdict = match hits.len() {
            0 => "no match".to_string(),
            1 => format!("MATCH → {}", hits[0].id),
            n => format!("AMBIGUOUS — {n} sessions share this value"),
        };
        println!("  {n:<2} {label:<14} {var:<8} {local_value:<40} {verdict}");
        if hits.len() == 1 && winner.is_none() {
            winner = Some((n, var, hits[0].id.clone()));
        }
    }

    let Some((n, var, session_id)) = winner else {
        println!();
        bail!(
            "no identity candidate joins this pane to an API session.\n\
             This is the escalation Phase 1's gate names: §2.4's status join and §2.5's\n\
             navigation both rest on such a key existing, so the product as designed is not\n\
             buildable until one is found. Do not carry this into Phase 2."
        );
    };

    let s = sessions.iter().find(|s| s.id == session_id).expect("winner came from this list");
    println!();
    println!("join key: `{var}` (candidate {n})");
    println!("resolves to session {session_id}");
    println!("  tty      {}", s.var("tty").unwrap_or("-"));
    println!("  path     {}", s.var("path").unwrap_or("-"));
    println!("  jobName  {}", s.var("jobName").unwrap_or("-"));
    if s.var("id") != Some(session_id.as_str()) {
        println!(
            "  note: the session's `id` variable ({}) differs from its ListSessions \
             unique_identifier ({session_id})",
            s.var("id").unwrap_or("-")
        );
    }
    Ok(Joined { session_id })
}

// ---------------------------------------------------------------------------------------
// Step 3 — activation
// ---------------------------------------------------------------------------------------

fn activate(session_id: &str) -> Result<()> {
    let mut client = Client::connect()?;
    let resp = client.call(Req::ActivateRequest(ActivateRequest {
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
        Ok(api::activate_response::Status::Ok) => {
            println!("activated {session_id}");
            Ok(())
        }
        Ok(status) => bail!("activate failed: {status:?}"),
        Err(_) => bail!("activate returned an unknown status: {:?}", r.status),
    }
}

// ---------------------------------------------------------------------------------------
// OQ-3 — what a variable-change subscription actually delivers
// ---------------------------------------------------------------------------------------

/// Outside the three things the phase's scope lists, and deliberately: the close-out has to
/// record what was observed about subscribing to variable changes, and that cannot be
/// satisfied without code that subscribes.
fn watch() -> Result<()> {
    let local = Local::read();
    let mut client = Client::connect()?;
    let sessions = client.sessions()?;
    let joined = report_identity(&local, &sessions)?;
    let window = sessions
        .iter()
        .find(|s| s.id == joined.session_id)
        .map(|s| s.window_id.clone())
        .expect("the joined session came out of this list");
    let mine: Vec<String> =
        sessions.iter().filter(|s| s.window_id == window).map(|s| s.id.clone()).collect();

    println!();
    println!("── subscriptions ────────────────────────────────────────────────────────");
    for session in &mine {
        for name in ["path", "jobName"] {
            client.subscribe(
                NotificationType::NotifyOnVariableChange,
                Some(VariableMonitorRequest {
                    name: Some(name.to_string()),
                    scope: Some(VariableScope::Session as i32),
                    identifier: Some(session.clone()),
                }),
                None,
            )?;
        }
        println!("watching path + jobName on {session}");
    }
    // These three ignore the session parameter and are posted on every such event.
    for t in [
        NotificationType::NotifyOnNewSession,
        NotificationType::NotifyOnTerminateSession,
        NotificationType::NotifyOnLayoutChange,
    ] {
        client.subscribe(t, None, None)?;
        println!("watching {t:?}");
    }

    println!();
    println!("waiting for notifications — cd in another tab, or open and close one. Ctrl-C to stop.");
    client.stream.get_mut().set_read_timeout(None)?;
    let start = std::time::Instant::now();
    loop {
        let msg = client.read_message()?;
        if let Some(Resp::Notification(n)) = msg.submessage {
            let at = start.elapsed().as_secs_f64();
            if let Some(v) = n.variable_changed_notification {
                println!(
                    "[{at:7.3}s] variable  {:<8} = {:<40} on {}",
                    v.name.unwrap_or_default(),
                    v.json_new_value.unwrap_or_default(),
                    v.identifier.unwrap_or_default()
                );
            } else if let Some(v) = n.new_session_notification {
                println!("[{at:7.3}s] new session      {}", v.session_id.unwrap_or_default());
            } else if let Some(v) = n.terminate_session_notification {
                println!("[{at:7.3}s] session ended    {}", v.session_id.unwrap_or_default());
            } else if n.layout_changed_notification.is_some() {
                println!("[{at:7.3}s] layout changed");
            } else {
                println!("[{at:7.3}s] other notification: {n:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// The transport itself — OQ-1 candidate 1, speaking the protocol directly from Rust
// ---------------------------------------------------------------------------------------

struct Client {
    stream: WebSocket<UnixStream>,
    next_id: i64,
}

/// One session as the probe needs it: what `ListSessions` gave, plus the variables that
/// only a per-session `VariableRequest` can return.
struct Session {
    id: String,
    window_id: String,
    vars: HashMap<String, String>,
}

impl Session {
    fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}

impl Client {
    fn connect() -> Result<Self> {
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
            let (cookie, key) = request_cookie_and_key()?;
            match Self::handshake(&path, &cookie, &key, library_version) {
                Ok(stream) => {
                    eprintln!("connected: {} (x-iterm2-library-version: {library_version})", path.display());
                    return Ok(Client { stream, next_id: 1 });
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
    ) -> Result<WebSocket<UnixStream>> {
        let socket = UnixStream::connect(path)
            .with_context(|| format!("connecting to {}", path.display()))?;
        socket.set_read_timeout(Some(Duration::from_secs(15)))?;

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
            .header("x-iterm2-advisory-name", ADVISORY_NAME)
            .body(())?;

        let (stream, response) = tungstenite::client(request, socket).map_err(|e| match e {
            tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) => {
                let body = r.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default().to_string();
                anyhow!("iTerm2 refused the handshake: HTTP {} {body}", r.status())
            }
            other => anyhow!("handshake failed: {other}"),
        })?;

        if let Some(v) = response.headers().get("X-iTerm2-Protocol-Version") {
            eprintln!("iTerm2 API protocol version: {}", v.to_str().unwrap_or("?"));
        }
        Ok(stream)
    }

    /// Sends one request and returns the response carrying the same id. Notifications, which
    /// arrive unsolicited and carry no id, are skipped.
    fn call(&mut self, submessage: Req) -> Result<Resp> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = ClientOriginatedMessage { id: Some(id), submessage: Some(submessage) };
        self.stream.send(WsMessage::binary(msg.encode_to_vec()))?;

        loop {
            let response = self.read_message()?;
            if response.id != Some(id) {
                continue;
            }
            return match response.submessage {
                Some(Resp::Error(e)) => bail!("iTerm2 rejected the request: {e}"),
                Some(other) => Ok(other),
                None => bail!("iTerm2 returned an empty response"),
            };
        }
    }

    fn read_message(&mut self) -> Result<ServerOriginatedMessage> {
        loop {
            match self.stream.read()? {
                WsMessage::Binary(b) => {
                    return ServerOriginatedMessage::decode(&b[..])
                        .context("decoding a ServerOriginatedMessage");
                }
                WsMessage::Close(frame) => bail!("iTerm2 closed the connection: {frame:?}"),
                // Ping/Pong are answered by tungstenite; Text is not something this API sends.
                _ => continue,
            }
        }
    }

    fn subscribe(
        &mut self,
        notification_type: NotificationType,
        variable_monitor: Option<VariableMonitorRequest>,
        session: Option<String>,
    ) -> Result<()> {
        let resp = self.call(Req::NotificationRequest(NotificationRequest {
            session,
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

    /// Every session iTerm2 knows about, in every window, with `WANTED_VARS` filled in.
    ///
    /// `ListSessions` alone is not enough: its `SessionSummary` carries only an identifier,
    /// a frame, a grid size and a title — neither the directory nor the job name §2.2 needs.
    /// Those take one `VariableRequest` per session.
    fn sessions(&mut self) -> Result<Vec<Session>> {
        let resp = self.call(Req::ListSessionsRequest(ListSessionsRequest {}))?;
        let Resp::ListSessionsResponse(list) = resp else {
            bail!("expected a list-sessions response, got {resp:?}");
        };

        let mut out = Vec::new();
        for (id, window_id) in flatten(&list) {
            let vars = self.variables(&id)?;
            out.push(Session { id, window_id, vars });
        }
        Ok(out)
    }

    fn variables(&mut self, session_id: &str) -> Result<HashMap<String, String>> {
        let resp = self.call(Req::VariableRequest(VariableRequest {
            scope: Some(api::variable_request::Scope::SessionId(session_id.to_string())),
            set: vec![],
            get: WANTED_VARS.iter().map(|s| s.to_string()).collect(),
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
        for (name, raw) in WANTED_VARS.iter().zip(&r.values) {
            match serde_json::from_str::<serde_json::Value>(raw) {
                Ok(serde_json::Value::Null) => {}
                Ok(serde_json::Value::String(s)) => {
                    vars.insert(name.to_string(), s);
                }
                Ok(other) => {
                    vars.insert(name.to_string(), other.to_string());
                }
                Err(_) => {
                    vars.insert(name.to_string(), raw.clone());
                }
            }
        }
        Ok(vars)
    }
}

/// Walks windows → tabs → split tree, yielding `(session id, window id)`.
///
/// Split panes are separate sessions here, which is §2.8's decision showing up in the
/// simplest place it can: a tab is a tree of sessions, not a session.
fn flatten(list: &ListSessionsResponse) -> Vec<(String, String)> {
    fn walk(node: &SplitTreeNode, out: &mut Vec<SessionSummary>) {
        for link in &node.links {
            match &link.child {
                Some(api::split_tree_node::split_tree_link::Child::Session(s)) => out.push(s.clone()),
                Some(api::split_tree_node::split_tree_link::Child::Node(n)) => walk(n, out),
                None => {}
            }
        }
    }

    let mut out = Vec::new();
    for window in &list.windows {
        let window_id = window.window_id.clone().unwrap_or_default();
        for tab in &window.tabs {
            let mut summaries = Vec::new();
            if let Some(root) = &tab.root {
                walk(root, &mut summaries);
            }
            summaries.extend(tab.minimized_sessions.iter().cloned());
            for s in summaries {
                if let Some(id) = s.unique_identifier {
                    out.push((id, window_id.clone()));
                }
            }
        }
    }
    // Buried sessions belong to no window and are deliberately left out.
    out
}

fn socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let suite = std::env::var("IT2_SUITE").unwrap_or_else(|_| "iTerm2".to_string());
    PathBuf::from(home).join("Library/Application Support").join(suite).join("private/socket")
}

/// iTerm2 hands out a per-process cookie and key over AppleScript, which is where both
/// authorization prompts live: macOS's Automation grant, then iTerm2's own "Allow Python
/// API Usage?" naming `ADVISORY_NAME`. The pair comes back space-separated.
fn request_cookie_and_key() -> Result<(String, String)> {
    let script =
        format!(r#"tell application "iTerm2" to request cookie and key for app named "{ADVISORY_NAME}""#);
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("running osascript to request an API cookie")?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!(
            "iTerm2 would not issue an API cookie: {}\n\
             If this is a permissions failure, grant Automation access when macOS asks, and\n\
             see rules/iterm-api.md for how to reset a grant that was denied once.",
            err.trim()
        );
    }

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    text.split_once(' ')
        .map(|(c, k)| (c.to_string(), k.to_string()))
        .ok_or_else(|| anyhow!("expected `<cookie> <key>` from osascript, got {text:?}"))
}
