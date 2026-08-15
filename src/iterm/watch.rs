//! Rows, and how they stay true.
//!
//! One row per session (§2.8), scoped to the window Oko itself is in, ordered by tab index
//! then position within the tab. The rows track reality by **subscription, not polling**
//! (OQ-3), and it takes two kinds:
//!
//! - `NOTIFY_ON_VARIABLE_CHANGE`, per session *and* per variable, for `path` and `jobName`.
//!   A session that appears later is covered only if it is subscribed on arrival.
//! - `NOTIFY_ON_LAYOUT_CHANGE` for the shape of the window. Dragging a tab creates no
//!   session, terminates none and changes no session variable, so this is the only event
//!   that makes the `tab` column live — and its payload is a whole `ListSessionsResponse`,
//!   so the new shape arrives inside the notification.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Result, bail};

use super::api::{
    ListSessionsResponse, Notification, NotificationType, SessionSummary, SplitTreeNode,
};
use super::client::{Client, decode_json_value};

/// The two variables a plain row is made of (§2.2).
const ROW_VARS: [&str; 2] = ["path", "jobName"];

/// How long the watcher waits for a notification before looking at its command channel.
/// Not a poll of iTerm2 — nothing is asked for; it is how fast a keystroke is served.
const IDLE_TICK: Duration = Duration::from_millis(100);

/// One line of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub session_id: String,
    /// 1-based position of the tab in its window's `tabs[]` — the only tab numbering the
    /// API offers (§2.8). Two sessions of a split tab share it.
    pub tab: usize,
    /// `jobName`, as the API reports it: iTerm2 truncates it to 16 bytes and Oko displays
    /// what it is given rather than repairing it.
    pub process: Option<String>,
    pub path: Option<String>,
}

/// Everything the table shows, at one moment.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub window_number: Option<i32>,
    pub rows: Vec<Row>,
}

/// What the socket thread tells the UI.
#[derive(Clone, Debug)]
pub enum Event {
    Snapshot(Snapshot),
    Error(String),
}

/// What the UI asks the socket thread for.
#[derive(Clone, Debug)]
pub enum Cmd {
    Activate(String),
}

/// A session and where it sits: which window, and which tab of it.
#[derive(Clone, Debug)]
pub struct Placed {
    pub session_id: String,
    pub window_id: String,
    pub window_number: Option<i32>,
    pub tab: usize,
}

pub struct Watcher {
    client: Client,
    own_session: String,
    window_id: String,
    window_number: Option<i32>,
    rows: Vec<Row>,
    /// Which (session, variable) pairs are already subscribed. A session that leaves this
    /// window and comes back is still subscribed — resubscribing it would be a second
    /// notification for every change.
    subscribed: HashSet<(String, &'static str)>,
}

impl Watcher {
    /// Connects, works out which session and window are ours, builds the first rows, and
    /// subscribes. Everything that can fail with a message worth reading — the API being
    /// off, no identity join — fails here, before any TUI exists to hide it.
    pub fn connect(advisory_name: &str) -> Result<Self> {
        let mut client = Client::connect(advisory_name)?;
        let list = client.list_sessions()?;
        let own_session = resolve_own_session(&mut client, &list)?;

        let mut watcher = Watcher {
            client,
            own_session,
            window_id: String::new(),
            window_number: None,
            rows: Vec::new(),
            subscribed: HashSet::new(),
        };
        watcher.rescan(&list)?;
        watcher.client.subscribe(NotificationType::NotifyOnLayoutChange, None)?;
        watcher.client.set_read_timeout(IDLE_TICK)?;
        Ok(watcher)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot { window_number: self.window_number, rows: self.rows.clone() }
    }

    pub fn own_session(&self) -> &str {
        &self.own_session
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.client.protocol_version()
    }

    /// Serves the socket until the UI goes away or the connection does. Owns the client for
    /// the rest of the program's life: one connection, because a cookie is spent by the
    /// connection that used it.
    ///
    /// `emit` returns false when the consumer is gone, which is the signal to stop. It is a
    /// closure rather than a channel so that this module owes nothing to how the UI names
    /// its events.
    pub fn run(mut self, cmds: &Receiver<Cmd>, mut emit: impl FnMut(Event) -> bool) {
        loop {
            // Commands first: a keystroke should not wait behind an idle read.
            loop {
                match cmds.try_recv() {
                    Ok(Cmd::Activate(id)) => {
                        if let Err(e) = self.client.activate(&id)
                            && !emit(Event::Error(format!("jump failed: {e:#}")))
                        {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    // The UI is gone; so is the reason to hold the socket.
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            match self.client.next_notification() {
                Ok(Some(n)) => match self.apply(&n) {
                    Ok(true) => {
                        if !emit(Event::Snapshot(self.snapshot())) {
                            return;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        emit(Event::Error(format!("{e:#}")));
                        return;
                    }
                },
                Ok(None) => {}
                Err(e) => {
                    emit(Event::Error(format!("connection lost: {e:#}")));
                    return;
                }
            }
        }
    }

    /// Folds one notification into the rows. `Ok(true)` when something the table shows
    /// changed.
    fn apply(&mut self, n: &Notification) -> Result<bool> {
        if let Some(v) = &n.variable_changed_notification {
            let (Some(id), Some(name)) = (&v.identifier, &v.name) else {
                return Ok(false);
            };
            let value = v.json_new_value.as_deref().and_then(decode_json_value);
            let Some(row) = self.rows.iter_mut().find(|r| &r.session_id == id) else {
                // A session of another window, or one that has already left ours.
                return Ok(false);
            };
            let field = match name.as_str() {
                "path" => &mut row.path,
                "jobName" => &mut row.process,
                _ => return Ok(false),
            };
            if *field == value {
                return Ok(false);
            }
            *field = value;
            return Ok(true);
        }

        if let Some(layout) = &n.layout_changed_notification {
            let Some(list) = &layout.list_sessions_response else {
                return Ok(false);
            };
            return self.rescan(list);
        }

        Ok(false)
    }

    /// Rebuilds the rows from a session list. Returns whether they changed.
    ///
    /// The window is resolved *every time*, as the one containing our own session, so
    /// dragging Oko's own tab into another window re-scopes the table rather than freezing
    /// it on a window we have left.
    fn rescan(&mut self, list: &ListSessionsResponse) -> Result<bool> {
        let placed = flatten(list);
        let Some(me) = placed.iter().find(|p| p.session_id == self.own_session) else {
            // Our own session is not in the list: it is closing, or iTerm2 sent a shape we
            // are not in. Keep the last rows rather than blanking the table.
            return Ok(false);
        };
        self.window_id = me.window_id.clone();
        self.window_number = me.window_number;

        let mut rows = Vec::new();
        for p in placed.iter().filter(|p| p.window_id == self.window_id) {
            // Values already held stay: a rescan is about shape, and re-reading every
            // variable on every layout change would be a poll wearing a subscription's hat.
            let known = self
                .rows
                .iter()
                .find(|r| r.session_id == p.session_id)
                .map(|r| (r.process.clone(), r.path.clone()));
            let (process, path) = match known {
                Some(values) => values,
                None => {
                    let vars = self.client.variables(&p.session_id, &ROW_VARS)?;
                    (vars.get("jobName").cloned(), vars.get("path").cloned())
                }
            };
            rows.push(Row { session_id: p.session_id.clone(), tab: p.tab, process, path });
        }

        for row in &rows {
            for var in ROW_VARS {
                if self.subscribed.insert((row.session_id.clone(), var)) {
                    self.client.watch_variable(&row.session_id, var)?;
                }
            }
        }

        let changed = rows != self.rows;
        self.rows = rows;
        Ok(changed)
    }
}

/// Which API session is the pane this process runs in.
///
/// §2.1's ordered fallback, and it still carries all three: Phase 1 confirmed the UUID
/// joins, and confirmed the other two join as well, on four sessions. Each candidate is a
/// membership test — the value is held locally and looked for in what the API reports — so
/// none of them needs the answer in order to ask the question.
pub fn resolve_own_session(client: &mut Client, list: &ListSessionsResponse) -> Result<String> {
    let term_session_id =
        std::env::var("TERM_SESSION_ID").or_else(|_| std::env::var("ITERM_SESSION_ID")).ok();
    // Observed shape: `w0t2p0:F79BC39C-…` — a window/tab/pane triple, a colon, then a UUID.
    let uuid = term_session_id.as_deref().and_then(|s| s.split_once(':')).map(|(_, u)| u.to_string());
    let placed = flatten(list);

    // 1. The UUID against `ListSessions`' `unique_identifier`, which is the same string as
    //    the session's `id` variable — the join key Phase 1 established.
    if let Some(uuid) = &uuid
        && placed.iter().any(|p| &p.session_id == uuid)
    {
        return Ok(uuid.clone());
    }

    // 2. The UUID against each session's `id` variable. Reached only if the two identifiers
    //    ever come apart — the evidence that they do not is four sessions wide.
    if let Some(uuid) = &uuid {
        for p in &placed {
            if client.variables(&p.session_id, &["id"])?.get("id") == Some(uuid) {
                return Ok(p.session_id.clone());
            }
        }
    }

    // 3. The tty device. The `wNtMpK` prefix is deliberately not tried: it is a position,
    //    and `t` is a monotonic id rather than a tab number (§2.8).
    if let Some(tty) = own_tty() {
        for p in &placed {
            if client.variables(&p.session_id, &["tty"])?.get("tty") == Some(&tty) {
                return Ok(p.session_id.clone());
            }
        }
    }

    if term_session_id.is_none() {
        bail!(
            "TERM_SESSION_ID is unset, so this is not running inside an iTerm2 pane.\n\
             Oko shows the window it is itself a tab of; start it from a tab of that window."
        );
    }
    bail!(
        "no identity candidate joins this pane to an API session.\n\
         The status join (§2.4) and Enter-to-jump (§2.5) both rest on such a key existing,\n\
         so this is an escalation rather than a fallback: see rules/iterm-api.md."
    );
}

/// The pane's tty device, as `/dev/ttysNNN`.
///
/// Asked about the standard descriptors, not about a freshly opened `/dev/tty`: that is a
/// cloning device with a device number of its own, so `ttyname` on it answers `/dev/tty` —
/// true, useless, and never equal to what the API reports.
pub fn own_tty() -> Option<String> {
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

/// Walks windows → tabs → split tree, in the order the API lists them.
///
/// Split panes are separate sessions here, which is §2.8's decision showing up in the
/// simplest place it can: a tab is a tree of sessions, not a session. The tab number is the
/// 1-based position in `tabs[]`, because the API exposes no tab index anywhere.
pub fn flatten(list: &ListSessionsResponse) -> Vec<Placed> {
    fn walk(node: &SplitTreeNode, out: &mut Vec<SessionSummary>) {
        for link in &node.links {
            match &link.child {
                Some(super::api::split_tree_node::split_tree_link::Child::Session(s)) => {
                    out.push(s.clone());
                }
                Some(super::api::split_tree_node::split_tree_link::Child::Node(n)) => walk(n, out),
                None => {}
            }
        }
    }

    let mut out = Vec::new();
    for window in &list.windows {
        let window_id = window.window_id.clone().unwrap_or_default();
        for (index, tab) in window.tabs.iter().enumerate() {
            let mut summaries = Vec::new();
            if let Some(root) = &tab.root {
                walk(root, &mut summaries);
            }
            summaries.extend(tab.minimized_sessions.iter().cloned());
            for s in summaries {
                if let Some(session_id) = s.unique_identifier {
                    out.push(Placed {
                        session_id,
                        window_id: window_id.clone(),
                        window_number: window.number,
                        tab: index + 1,
                    });
                }
            }
        }
    }
    // Buried sessions belong to no window and are deliberately left out.
    out
}

/// The variables a row is built from, for callers that want to read them directly.
pub fn row_variables(client: &mut Client, session_id: &str) -> Result<HashMap<String, String>> {
    client.variables(session_id, &ROW_VARS)
}
