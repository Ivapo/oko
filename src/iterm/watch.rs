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
//!
//! The status column has no subscription to hang on, because it comes from a directory of
//! files rather than from iTerm2. It rides the tick this loop already wakes on: one `stat`
//! of `~/.oko/status`, a re-read only when the mtime moved, and one comparison of the
//! merged view — which is also the only thing that can notice a `working` going stale, since
//! ageing writes no file and fires nothing.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use anyhow::{Result, bail};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::api::{
    ListSessionsResponse, Notification, NotificationType, SessionSummary, SplitTreeNode,
};
use super::client::{Client, decode_json_value};
use crate::status::{Shown, Store};

/// The variables a row is made of: the two §2.2 names, and the name §2.10 stores.
///
/// One array, because it drives three things that must not drift apart — what `rescan`
/// fetches for a session it has not seen, what it subscribes that session to, and what
/// [`row_variables`] hands a caller. `user.okoName` must be spelled with that prefix:
/// iTerm2 answers `INVALID_NAME` for a `user.`-less name a client tries to set.
const ROW_VARS: [&str; 3] = ["path", "jobName", OKO_NAME];

/// Where a row's name lives — a variable on the iTerm2 session itself (§2.10).
///
/// A session variable rather than a file, and the reason is that Phase 3 already paid for
/// the lesson: a file keyed by session id needs its own sweep, its own answer to "what if
/// the pane is gone", and its own concurrent-write discipline. This needs none of them. It
/// dies with the pane, iTerm2 owns the concurrency, and two Oko instances see one value
/// with no sync protocol at all.
pub const OKO_NAME: &str = "user.okoName";

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
    /// what it is given rather than repairing it. **A row carrying a status shows the
    /// literal `claude` instead** (OQ-2) — that is `src/ui.rs`'s doing, not this field's.
    pub process: Option<String>,
    pub path: Option<String>,
    /// The row's stored name: `user.okoName`, or `None` when nobody has named it.
    ///
    /// A variable like `path` and `jobName`, held here and patched by the same notification
    /// path. **An explicit rename is stored and is then the name**, through any subsequent
    /// `cd`: at that point a human has said what this row *is*, which a directory change
    /// does not invalidate.
    pub stored_name: Option<String>,
    /// What the `name` column draws: [`stored_name`], else the last component of [`path`].
    ///
    /// **Filled at snapshot time only**, like [`status`] and for a sharper reason: the
    /// derived default is *not* a name, it is a description of where the row currently is,
    /// so it has to be recomputed as the row moves. Storing it at first sight would freeze
    /// it to whatever directory the pane happened to be in when Oko started, which is the
    /// wrong answer for a shell a human navigates.
    ///
    /// [`stored_name`]: Row::stored_name
    /// [`path`]: Row::path
    /// [`status`]: Row::status
    pub name: Option<String>,
    /// What Claude Code last said about this session and how long ago, if it is a Claude tab
    /// at all.
    ///
    /// **Filled at snapshot time only.** The watcher's own rows always carry `None` here:
    /// shape and variables come from iTerm2 and status comes from a directory of files, and
    /// keeping them apart until the last moment is what lets one comparison of the merged
    /// view speak for all three — including a `working` going stale or an age crossing a
    /// bucket boundary, neither of which any notification or file write ever announces.
    pub status: Option<Shown>,
}

/// Everything the table shows, at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
///
/// Both variants exist because the UI cannot reach iTerm2 itself: `src/main.rs:run` moves
/// the `Watcher`, and with it the only `Client`, into the socket thread.
#[derive(Clone, Debug)]
pub enum Cmd {
    Activate(String),
    /// Set a session's name, or clear it with `None` — which is the only way back to the
    /// derived default.
    Rename(String, Option<String>),
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
    /// Shape and variables. Never carries a status — see [`Row::status`].
    rows: Vec<Row>,
    /// Which (session, variable) pairs are already subscribed. A session that leaves this
    /// window and comes back is still subscribed — resubscribing it would be a second
    /// notification for every change.
    subscribed: HashSet<(String, &'static str)>,
    /// What the hooks have written, and the mtime that says whether to look again.
    status: Store,
    /// The last snapshot handed to the UI, so nothing is sent twice.
    emitted: Snapshot,
    /// Where to record each emission, under `OKO_DEBUG_EMITS`. See [`log_emit`].
    emits_log: Option<PathBuf>,
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
            status: Store::open(),
            emitted: Snapshot::default(),
            emits_log: emits_log(),
        };
        // Before the first rescan, so its sweep runs against statuses that were read: a pane
        // that died while Oko was closed is cleaned up here, where no hook ever ran for it.
        watcher.status.refresh();
        watcher.rescan(&list)?;
        // Both, because neither covers the other: closing a tab fires the
        // layout change, opening one fires only the new-session.
        watcher.client.subscribe(NotificationType::NotifyOnLayoutChange, None)?;
        watcher.client.subscribe(NotificationType::NotifyOnNewSession, None)?;
        watcher.client.set_read_timeout(IDLE_TICK)?;
        watcher.emitted = watcher.snapshot();
        Ok(watcher)
    }

    /// The rows as the table should draw them: shape and variables from `self.rows`, status
    /// merged in from the hooks, and the name resolved. The one place the three meet.
    pub fn snapshot(&self) -> Snapshot {
        let now = OffsetDateTime::now_utc();
        let rows = self
            .rows
            .iter()
            .map(|row| Row {
                name: row.stored_name.clone().or_else(|| last_component(row.path.as_deref())),
                status: self.status.status_of(&row.session_id, now),
                ..row.clone()
            })
            .collect();
        Snapshot { window_number: self.window_number, rows }
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
                    Ok(Cmd::Rename(id, name)) => {
                        if let Err(e) = self.rename(&id, name)
                            && !emit(Event::Error(format!("rename failed: {e:#}")))
                        {
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    // The UI is gone; so is the reason to hold the socket.
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            // The status directory, on the same tick. One `stat`, and a re-read only when
            // the mtime moved — **not** the polling OQ-3 ruled out, which is about iTerm2's
            // API and still pushes.
            self.status.refresh();

            match self.client.next_notification() {
                Ok(Some(n)) => {
                    if let Err(e) = self.apply(&n) {
                        emit(Event::Error(format!("{e:#}")));
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    emit(Event::Error(format!("connection lost: {e:#}")));
                    return;
                }
            }

            if !self.emit_if_changed(&mut emit) {
                return;
            }
        }
    }

    /// The single emission point: builds the merged view and sends it only when it differs
    /// from the last one sent. Returns false when the consumer is gone.
    ///
    /// One comparison rather than one per source, because the sources disagree about what
    /// "changed" means. A variable change and a layout change each know they moved
    /// something; **a `working` crossing `OKO_STALE_AFTER` knows nothing at all** — no file
    /// is written and no notification fires, only the clock moves. Comparing the merged view
    /// on every tick is the only thing that catches all three.
    fn emit_if_changed(&mut self, emit: &mut impl FnMut(Event) -> bool) -> bool {
        let snapshot = self.snapshot();
        if snapshot == self.emitted {
            return true;
        }
        self.emitted = snapshot.clone();
        log_emit(self.emits_log.as_deref());
        emit(Event::Snapshot(snapshot))
    }

    /// Sets a row's name on the session itself, or clears it.
    ///
    /// The watcher's own row is updated here rather than in the UI, and that is not a
    /// preference: the UI's snapshot is a copy, so a name written into it would show for one
    /// frame and vanish at the next emission — which looks like a flaky rename rather than
    /// the misplaced write it is. The subscription in `rescan` would eventually deliver the
    /// same value, but waiting for the round trip is a visible stutter.
    fn rename(&mut self, session_id: &str, name: Option<String>) -> Result<()> {
        // JSON, because that is what the API speaks in both directions, and `null` is what
        // unsets — measured (OQ-5) to read back *absent* rather than as an empty string.
        // `""` would decode through `decode_json_value` as `Some("")` and render a blank
        // name no further rename could escape.
        let value = match &name {
            Some(name) => serde_json::Value::String(name.clone()).to_string(),
            None => "null".to_string(),
        };
        self.client.set_variable(session_id, OKO_NAME, &value)?;
        if let Some(row) = self.rows.iter_mut().find(|r| r.session_id == session_id) {
            row.stored_name = name;
        }
        Ok(())
    }

    /// Folds one notification into the rows.
    ///
    /// Whether anything the table shows actually moved is not decided here — that is
    /// [`emit_if_changed`](Self::emit_if_changed)'s single comparison, which sees status and
    /// staleness as well as shape.
    fn apply(&mut self, n: &Notification) -> Result<()> {
        if let Some(v) = &n.variable_changed_notification {
            let (Some(id), Some(name)) = (&v.identifier, &v.name) else {
                return Ok(());
            };
            let value = v.json_new_value.as_deref().and_then(decode_json_value);
            let Some(row) = self.rows.iter_mut().find(|r| &r.session_id == id) else {
                // A session of another window, or one that has already left ours.
                return Ok(());
            };
            match name.as_str() {
                "path" => row.path = value,
                "jobName" => row.process = value,
                // Which is how a rename made in *another* Oko instance arrives here: one
                // value on the session, seen by every client watching it, no sync protocol.
                OKO_NAME => row.stored_name = value,
                _ => {}
            }
            return Ok(());
        }

        if let Some(layout) = &n.layout_changed_notification
            && let Some(list) = &layout.list_sessions_response
        {
            self.rescan(list)?;
            return Ok(());
        }

        // A tab **opening** fires only this one — measured, not inferred: iTerm2
        // sends `NotifyOnNewSession` for it and no layout change, while closing
        // one sends the layout change and no new-session. Subscribing to layout
        // alone is why a new tab used to never appear while a closed one left
        // at once. It carries an id and nothing else, so the list is fetched.
        if n.new_session_notification.is_some() {
            let list = self.client.list_sessions()?;
            self.rescan(&list)?;
        }

        Ok(())
    }

    /// Rebuilds the rows from a session list, and sweeps the status files of sessions that
    /// no longer exist.
    ///
    /// The window is resolved *every time*, as the one containing our own session, so
    /// dragging Oko's own tab into another window re-scopes the table rather than freezing
    /// it on a window we have left.
    fn rescan(&mut self, list: &ListSessionsResponse) -> Result<()> {
        let placed = flatten(list);
        self.sweep_status(list, &placed);

        let Some(me) = placed.iter().find(|p| p.session_id == self.own_session) else {
            // Our own session is not in the list: it is closing, or iTerm2 sent a shape we
            // are not in. Keep the last rows rather than blanking the table.
            return Ok(());
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
                .map(|r| (r.process.clone(), r.path.clone(), r.stored_name.clone()));
            let (process, path, stored_name) = match known {
                Some(values) => values,
                None => {
                    let vars = self.client.variables(&p.session_id, &ROW_VARS)?;
                    (
                        vars.get("jobName").cloned(),
                        vars.get("path").cloned(),
                        vars.get(OKO_NAME).cloned(),
                    )
                }
            };
            // `name` and `status` are `None` always: both are merged in by `snapshot` and
            // neither is ever held here. For the name that is what keeps the derived default
            // derived — a stored one would stop following a `cd`.
            rows.push(Row {
                session_id: p.session_id.clone(),
                tab: p.tab,
                process,
                path,
                stored_name,
                name: None,
                status: None,
            });
        }

        for row in &rows {
            for var in ROW_VARS {
                if self.subscribed.insert((row.session_id.clone(), var)) {
                    self.client.watch_variable(&row.session_id, var)?;
                }
            }
        }

        self.rows = rows;
        Ok(())
    }

    /// Deletes the status file of any session iTerm2 no longer has (OQ-4 (b)).
    ///
    /// Two things about the scope are load-bearing. It is **every window of the whole
    /// response**, not Oko's rows: rows are window-scoped, so sweeping against them would
    /// destroy the live status of Claude tabs in other windows, and two Okos in two windows
    /// would delete each other's files continuously. And **buried sessions count as alive**
    /// — they sit outside `windows[]`, so [`flatten`] drops them deliberately, and "in no
    /// window" is true of a buried but perfectly healthy Claude session.
    ///
    /// It runs here rather than on the status tick because a closing tab writes no file, so
    /// an mtime-gated tick would never fire it; a layout change is exactly the event that
    /// says a session went away, and it carries the whole session list with it.
    fn sweep_status(&mut self, list: &ListSessionsResponse, placed: &[Placed]) {
        let live: HashSet<&str> = placed
            .iter()
            .map(|p| p.session_id.as_str())
            .chain(list.buried_sessions.iter().filter_map(|s| s.unique_identifier.as_deref()))
            .collect();
        self.status.sweep(&live);
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
    let uuid =
        term_session_id.as_deref().and_then(|s| s.split_once(':')).map(|(_, u)| u.to_string());
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

/// The last component of a path — the derived default a row shows until it is named (§2.10).
///
/// Computed here rather than stored, so it follows a `cd`: it is not a name, it is a
/// description of where the row currently is.
fn last_component(path: Option<&str>) -> Option<String> {
    let path = path?;
    match path.rsplit('/').find(|part| !part.is_empty()) {
        Some(name) => Some(name.to_string()),
        // The root, which has no last component but is somewhere all the same.
        None if path.starts_with('/') => Some("/".to_string()),
        None => None,
    }
}

/// `~/.oko/emits.log`, or `None` unless `OKO_DEBUG_EMITS` is set.
fn emits_log() -> Option<PathBuf> {
    std::env::var_os("OKO_DEBUG_EMITS")?;
    crate::status::oko_dir().ok().map(|dir| dir.join("emits.log"))
}

/// One line per emission, for the check that Oko stays quiet.
///
/// §2.11's quietness is the one property a human watching the screen cannot verify — a
/// redundant redraw of identical content is invisible — so it is counted rather than
/// watched. **A log rather than a total reported on exit**, because the check needs two
/// readings sixty seconds apart *from one running process*: two runs would compare two
/// counters both starting at zero. Exit is no place to report from either, since
/// `src/main.rs:run` never joins this thread, and the alternate screen rules out `eprintln!`.
/// A file the running process appends to is readable with `wc -l` from another tab without
/// stopping anything.
///
/// **The line carries a timestamp and no row content.** Logging *what* was emitted is the
/// natural debugging instinct and row names are the interesting part — which would break the
/// gate check that greps `~/.oko/` for a name it expects to find nowhere on disk.
///
/// It cannot perturb what it measures: it changes no `Snapshot` field and does not touch
/// `~/.oko/status/`, whose mtime is what `src/status.rs:Store::refresh` gates on.
fn log_emit(path: Option<&Path>) {
    let Some(path) = path else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default();
        let _ = writeln!(file, "{now}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_derived_default_is_the_last_component() {
        assert_eq!(last_component(Some("/Users/me/dev/main/oko")).as_deref(), Some("oko"));
        // A trailing slash is still that directory, not an empty name.
        assert_eq!(last_component(Some("/Users/me/dev/main/")).as_deref(), Some("main"));
        assert_eq!(last_component(Some("/")).as_deref(), Some("/"));
        // No path at all: the row falls through to the `-` every other column uses.
        assert_eq!(last_component(None), None);
    }

    #[test]
    fn a_stored_name_wins_over_the_derived_one() {
        let row = |stored: Option<&str>, path: &str| Row {
            session_id: "S".to_string(),
            tab: 1,
            process: None,
            path: Some(path.to_string()),
            stored_name: stored.map(str::to_owned),
            name: None,
            status: None,
        };
        // What `snapshot` computes, as the expression it computes.
        let resolve = |r: &Row| r.stored_name.clone().or_else(|| last_component(r.path.as_deref()));

        assert_eq!(resolve(&row(None, "/Users/me/dev/main/oko")).as_deref(), Some("oko"));
        // A `cd` moves the derived default...
        assert_eq!(resolve(&row(None, "/Users/me/dev/main")).as_deref(), Some("main"));
        // ...and does not move a name a human set.
        assert_eq!(resolve(&row(Some("api work"), "/anywhere")).as_deref(), Some("api work"));
    }

    #[test]
    fn an_emission_logs_one_line_and_no_row_content() {
        let dir = std::env::temp_dir().join("oko-emits-test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("emits.log");

        log_emit(Some(&path));
        log_emit(Some(&path));
        let text = std::fs::read_to_string(&path).unwrap();

        // Two emissions, two lines — `wc -l` is the whole of the check.
        assert_eq!(text.lines().count(), 2);
        // A timestamp and nothing else. The gate greps `~/.oko/` for a row name it expects
        // to find nowhere, so logging *what* was emitted would break it against a correct
        // build — and row names are exactly the debugging instinct that would put them here.
        for line in text.lines() {
            OffsetDateTime::parse(line, &Rfc3339).expect("a bare RFC-3339 timestamp");
        }

        // Unset, and nothing is written at all.
        let quiet = dir.join("never.log");
        log_emit(None);
        assert!(!quiet.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
