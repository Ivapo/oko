//! The headless diagnostic. Phase 1 of `specs/tab_dashboard_spec.md` produced it as a
//! transport spike; Phase 2 kept it, rewired onto `oko::iterm` so there is exactly one
//! client, because when a gate check fails inside a full-screen TUI this is what can be
//! read.
//!
//!     probe                      identity, then the sessions of this window
//!     probe activate <session>   focus that session, its tab, and its window
//!     probe watch                print notifications as they arrive
//!
//! `watch` deliberately subscribes to **more** than the dashboard does — new-session and
//! terminate-session as well as layout-change — because its job is to show which
//! notification actually fires for an event, which is the one thing the dashboard's design
//! infers rather than measures.
//!
//! Setup — enabling the API, authorizing a client — is in `rules/iterm-api.md`.

use std::time::Instant;

use anyhow::{Result, anyhow, bail};

use oko::iterm::api::NotificationType;
use oko::iterm::{Client, flatten, own_tty, resolve_own_session};

/// Its own name, so the dashboard's authorization is never disturbed by a diagnostic run.
const ADVISORY_NAME: &str = "oko-probe";

/// Three identity candidates (§2.1) and the two values a plain row is made of (§2.2).
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
            let session =
                args.get(1).ok_or_else(|| anyhow!("usage: probe activate <session-id>"))?;
            let mut client = connect()?;
            client.activate(session)?;
            println!("activated {session}");
            Ok(())
        }
        Some("watch") => watch(),
        Some(other) => {
            bail!("unknown command {other:?}; expected `activate <session-id>` or `watch`")
        }
    }
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
        client.subscribe(notification, None)?;
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
