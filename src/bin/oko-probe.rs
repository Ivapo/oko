//! The headless diagnostic. Phase 1 of `specs/tab_dashboard_spec.md` produced it as a
//! transport spike; Phase 2 kept it, rewired onto `oko::iterm` so there is exactly one
//! client, because when a gate check fails inside a full-screen TUI this is what can be
//! read.
//!
//!     oko-probe                      identity, then the sessions of this window
//!     oko-probe activate <session>   focus that session, its tab, and its window
//!     oko-probe watch                print notifications as they arrive
//!     oko-probe hx [<session>]       what every Helix pane has open, and how it is read
//!     oko-probe screen-watch <s>…    one line per screen update, with the gap
//!
//! `watch` deliberately subscribes to **more** than the dashboard does — terminate-session
//! as well as layout-change and new-session — because its job is to show which notification
//! actually fires for an event, which is the one thing the dashboard's design infers rather
//! than measures. That was not an idle worry: the dashboard subscribed to layout-change
//! alone until `e05bf6a`, on an inference this command disproved — a tab *opening* fires
//! new-session and no layout change — so it is one subscription closer to this one now.
//!
//! Setup — enabling the API, authorizing a client — is in `rules/iterm-api.md`.

use std::time::Instant;

use anyhow::{Result, anyhow, bail};

// `status` is here although nothing below names it: `src/iterm/watch.rs` reaches
// `crate::status`, so the module `iterm` needs it even where this binary does not.
#[path = "../iterm/mod.rs"]
mod iterm;
#[path = "../status.rs"]
mod status;

use iterm::api::NotificationType;
use iterm::{Client, flatten, helix, own_tty, resolve_own_session};

/// Its own name, so the dashboard's authorization is never disturbed by a diagnostic run.
const ADVISORY_NAME: &str = "oko-probe";

/// The `//!` block's lines. `var` is deliberately absent: it is OQ-5's spike, kept as a
/// diagnostic, and not one of the things a person reaching for this binary wants. **`hx` and
/// `screen-watch` are here, and in the unknown-command message, for the opposite reason**:
/// they are how a later reader re-measures OQ-15 and OQ-16 against a Helix that has moved on.
const USAGE: &str = "\
oko-probe — Oko's headless diagnostic: what iTerm2 thinks, without a full-screen TUI in the
way. When a dashboard row looks wrong, this is what says whether iTerm2 ever reported it.

usage:
  oko-probe                       identity, then the sessions of this window
  oko-probe activate <session>    focus that session, its tab, and its window
  oko-probe watch                 print notifications as they arrive
  oko-probe hx [<session>]        every Helix pane: its job names, its launch arguments, the
                                  rows of its screen and the file the parser reads off them.
                                  With a session, that session's screen rows alone, verbatim,
                                  which is how a parser fixture is captured
  oko-probe screen-watch <s>...   subscribe those sessions to screen updates and print one
                                  line per update, with the gap since that session's last
  oko-probe --help, -h            print this

`watch` subscribes to more than the dashboard does, so it can tell you whether iTerm2 sent
an event at all — which is the difference between Oko missing something and iTerm2 not
saying it.";

/// Three identity candidates (§2.1) and the two values a plain row is made of (§2.2).
const WANTED_VARS: [&str; 5] = ["id", "tty", "termid", "path", "jobName"];

fn main() {
    if let Err(e) = run() {
        eprintln!("oko-probe: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => enumerate(),
        Some("activate") => {
            let session =
                args.get(1).ok_or_else(|| anyhow!("usage: oko-probe activate <session-id>"))?;
            let mut client = connect()?;
            client.activate(session)?;
            println!("activated {session}");
            Ok(())
        }
        Some("watch") => watch(),
        Some("hx") => hx(args.get(1).map(String::as_str)),
        Some("screen-watch") => screen_watch(&args[1..]),
        Some("var") => var_spike(),
        // It never fell through to enumerating a window — `Some(other)` below already
        // `bail!`s — but it answered with no usage text and exit 1, under a prefix this
        // phase is otherwise removing.
        Some("--help" | "-h") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            bail!(
                "unknown command {other:?}; expected `activate <session-id>`, `watch`, \
                 `hx [<session-id>]`, `screen-watch <session-id>...` or `var`"
            )
        }
    }
}

/// OQ-5's spike: can Oko set, read back and *watch* a `user.` variable on a session that is
/// not its own?
///
/// §2.10 stores a row's name in `user.okoName`, and nothing in this repo has ever written a
/// variable — Phase 1 only ever read, and only from its own window. Three things have to
/// hold and they are measured separately, because they fail for different reasons.
fn var_spike() -> Result<()> {
    const KEY: &str = "user.okoSpike";

    let mut client = connect()?;
    let list = client.list_sessions()?;
    let placed = flatten(&list);
    let own = resolve_own_session(&mut client, &list)?;

    // Deliberately not our own session: writing to a pane we occupy would prove the weaker
    // claim, and §2.10 needs the stronger one.
    let target = placed
        .iter()
        .find(|p| p.session_id != own)
        .ok_or_else(|| anyhow!("need a second session in some window to write to"))?;
    println!("own      {own}");
    println!("target   {}  (a session this process does not occupy)", target.session_id);
    println!();

    // 1. Set.
    let written = "phase-4 spike";
    client.set_variable(&target.session_id, KEY, &format!("{written:?}"))?;
    println!("1. set          {KEY} = {written:?}  → OK");

    // 2. Read back.
    let got = client.variables(&target.session_id, &[KEY])?.get(KEY).cloned();
    println!("2. read back    {got:?}");
    if got.as_deref() != Some(written) {
        bail!("read-back mismatch: expected {written:?}, got {got:?}");
    }

    // 3. Watch. Subscribe, then change it, and see whether a notification arrives.
    client.watch_variable(&target.session_id, KEY)?;
    client.set_read_timeout(std::time::Duration::from_millis(200))?;
    client.set_variable(&target.session_id, KEY, "\"changed\"")?;

    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    let mut seen = None;
    while Instant::now() < deadline && seen.is_none() {
        if let Some(n) = client.next_notification()?
            && let Some(v) = n.variable_changed_notification
            && v.name.as_deref() == Some(KEY)
        {
            seen = Some(format!("{:?} on {:?}", v.json_new_value, v.identifier));
        }
    }
    match &seen {
        Some(what) => println!("3. watch        notification arrived: {what}"),
        None => println!("3. watch        NO notification within 3s"),
    }

    // Leave nothing behind: null unsets.
    client.set_variable(&target.session_id, KEY, "null")?;
    let after = client.variables(&target.session_id, &[KEY])?.get(KEY).cloned();
    println!("4. unset (null) reads back as {after:?}  (None = gone)");

    // Set and read-back are assertions above — reaching here means they held.
    println!();
    println!("OQ-5: set=yes read=yes watch={}", if seen.is_some() { "yes" } else { "NO" });
    Ok(())
}

fn connect() -> Result<Client> {
    let client = Client::connect(ADVISORY_NAME)?;
    if let Some(version) = client.protocol_version() {
        eprintln!("connected: iTerm2 API protocol version {version}");
    }
    Ok(client)
}

/// Identity first, then the sessions of this window with the tab number the dashboard shows.
fn enumerate() -> Result<()> {
    let mut client = connect()?;
    let list = client.list_sessions()?;
    let placed = flatten(&list);

    println!("── identity ─────────────────────────────────────────────────────────────");
    println!(
        "own TERM_SESSION_ID   {}",
        std::env::var("TERM_SESSION_ID")
            .unwrap_or_else(|_| "(unset — not running inside an iTerm2 pane)".into())
    );
    println!(
        "own /dev/tty          {}",
        own_tty().unwrap_or_else(|| "(unavailable — no controlling terminal)".into())
    );
    println!();
    println!("  {:<38}  {:<14}  {:<10}  jobName", "id", "tty", "termid");
    for p in &placed {
        let vars = client.variables(&p.session_id, &WANTED_VARS)?;
        let get = |name: &str| vars.get(name).cloned().unwrap_or_else(|| "-".into());
        println!(
            "  {:<38}  {:<14}  {:<10}  {}",
            get("id"),
            get("tty"),
            get("termid"),
            get("jobName")
        );
    }

    let own = resolve_own_session(&mut client, &list)?;
    let me = placed
        .iter()
        .find(|p| p.session_id == own)
        .expect("the joined session came out of this list");
    println!();
    println!("joins to session {own}");

    println!();
    println!("── sessions in this window ──────────────────────────────────────────────");
    let mine: Vec<_> = placed.iter().filter(|p| p.window_id == me.window_id).collect();
    println!(
        "{} session(s) in window {} (number {})",
        mine.len(),
        me.window_id,
        me.window_number.map_or_else(|| "-".into(), |n| n.to_string())
    );
    println!("{:<4}  {:<38}  {:<17}  where", "tab", "session", "process");
    for p in &mine {
        let vars = client.variables(&p.session_id, &["path", "jobName"])?;
        let get = |name: &str| vars.get(name).cloned().unwrap_or_else(|| "-".into());
        println!("{:<4}  {:<38}  {:<17}  {}", p.tab, p.session_id, get("jobName"), get("path"));
    }

    let elsewhere = placed.len() - mine.len();
    if elsewhere > 0 {
        println!();
        println!("({elsewhere} further session(s) exist in other windows and are not listed)");
    }
    Ok(())
}

/// Subscribes to everything that could carry a change and prints what arrives, with a
/// timestamp, so an event can be attributed to a notification type by eye.
fn watch() -> Result<()> {
    let mut client = connect()?;
    let list = client.list_sessions()?;
    let placed = flatten(&list);
    let own = resolve_own_session(&mut client, &list)?;
    let me = placed.iter().find(|p| p.session_id == own).expect("the join came from this list");

    println!("── subscriptions ────────────────────────────────────────────────────────");
    for p in placed.iter().filter(|p| p.window_id == me.window_id) {
        for name in ["path", "jobName"] {
            client.watch_variable(&p.session_id, name)?;
        }
        println!("watching path + jobName on tab {} · {}", p.tab, p.session_id);
    }
    for notification in [
        NotificationType::NotifyOnNewSession,
        NotificationType::NotifyOnTerminateSession,
        NotificationType::NotifyOnLayoutChange,
    ] {
        client.subscribe(notification, None, None)?;
        println!("watching {notification:?}");
    }

    println!();
    println!("waiting — cd, split a pane, drag a tab, open or close one. Ctrl-C to stop.");
    let start = Instant::now();
    loop {
        let Some(n) = client.next_notification()? else {
            continue;
        };
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
        } else if let Some(v) = n.layout_changed_notification {
            let tabs = v
                .list_sessions_response
                .as_ref()
                .and_then(|l| l.windows.iter().find(|w| w.window_id == Some(me.window_id.clone())))
                .map_or(0, |w| w.tabs.len());
            println!("[{at:7.3}s] layout changed   this window now has {tabs} tab(s)");
        } else {
            println!("[{at:7.3}s] other notification: {n:?}");
        }
    }
}

/// The four variables §2.17 measured, in the order it names them. `jobName` is the gate key
/// (OQ-15); `deepestJob` is the one §2.2's sentence actually describes; `commandLine` and
/// `terminalWindowName` are the two candidates §2.17 rejected, printed so a later reader can
/// watch them stay at the launch arguments while the file changes.
const HX_VARS: [&str; 4] = ["jobName", "deepestJob", "commandLine", "terminalWindowName"];

/// What every Helix pane of this window has open — or, given a session, that session's screen
/// rows alone.
///
/// The two forms are one command because they answer one question at two altitudes. The
/// report is for a human asking why a row says what it says; the bare rows are how a parser
/// fixture is captured, byte for byte, which a report can never be. The operand deliberately
/// accepts **any** session, not only a Helix one: the stream carries a file only for a row
/// whose job is `hx` (§2.18), so pointed at a pane that is not one — Oko's own, a shell — this
/// is still the only oracle there is for what a screen actually holds.
fn hx(session: Option<&str>) -> Result<()> {
    let mut client = connect()?;

    // Verbatim, nothing else, no header: a fixture is worth exactly its being unedited.
    if let Some(session) = session {
        for row in client.screen(session)? {
            println!("{row}");
        }
        return Ok(());
    }

    let list = client.list_sessions()?;
    let placed = flatten(&list);
    let own = resolve_own_session(&mut client, &list)?;
    let me = placed.iter().find(|p| p.session_id == own).expect("the join came from this list");

    let mut found = 0;
    for p in placed.iter().filter(|p| p.window_id == me.window_id) {
        let vars = client.variables(&p.session_id, &HX_VARS)?;
        let get = |name: &str| vars.get(name).cloned().unwrap_or_else(|| "-".into());
        if get("jobName") != "hx" {
            continue;
        }
        found += 1;

        println!("── tab {} · {} ──────────────────────────────────", p.tab, p.session_id);
        for name in HX_VARS {
            println!("  {name:<20} {}", get(name));
        }

        let rows = client.screen(&p.session_id)?;
        println!("  {:<20} {} row(s)", "screen", rows.len());
        // `{:?}` rather than the bare text: it quotes and escapes, so trailing whitespace is
        // visible and unambiguous, and it does not wrap a row in the very character the
        // parser splits on.
        for (i, row) in rows.iter().enumerate() {
            println!("  {i:>3} {row:?}");
        }
        // What the dashboard would make of that screen, beside the screen itself: the two
        // together are the whole of "why does this row say what it says".
        println!("  {:<20} {:?}", "parser", helix::open_file(&rows));
        println!();
        println!("  capture it:  oko-probe hx {} > <fixture>.txt", p.session_id);
        println!();
    }

    if found == 0 {
        println!("no session in this window has jobName `hx`.");
    }
    Ok(())
}

/// One line per screen update, per session, with the gap since that session's last.
///
/// The gap is the point rather than a nicety: OQ-16's window was chosen from the gaps inside
/// a burst — 15–49 ms with a key held, 11–17 ms while a language server starts — so a command
/// that printed timestamps alone could confirm the window but never re-derive it.
fn screen_watch(sessions: &[String]) -> Result<()> {
    if sessions.is_empty() {
        bail!("usage: oko-probe screen-watch <session-id>...");
    }
    let mut client = connect()?;
    for session in sessions {
        client.watch_screen(session, true)?;
        println!("watching screen updates on {session}");
    }

    println!();
    println!("waiting — type in one of those panes, hold a key down, open a file. Ctrl-C to stop.");
    let start = Instant::now();
    let mut last: std::collections::HashMap<String, Instant> = std::collections::HashMap::new();
    loop {
        let Some(n) = client.next_notification()? else {
            continue;
        };
        let Some(update) = n.screen_update_notification else {
            continue;
        };
        let session = update.session.unwrap_or_default();
        let now = Instant::now();
        let gap = match last.insert(session.clone(), now) {
            Some(previous) => format!("{:8.0} ms", (now - previous).as_secs_f64() * 1000.0),
            None => "       — ms".to_string(),
        };
        println!("[{:7.3}s] {gap}  {session}", start.elapsed().as_secs_f64());
    }
}
