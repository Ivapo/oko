//! The headless diagnostic. Phase 1 of `specs/tab_dashboard_spec.md` produced it as a
//! transport spike; Phase 2 kept it, rewired onto `oko::iterm` so there is exactly one
//! client, because when a gate check fails inside a full-screen TUI this is what can be
//! read.
//!
//!     oko-probe                      identity, then the sessions of this window
//!     oko-probe activate <session>   focus that session, its tab, and its window
//!     oko-probe watch                print notifications as they arrive
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

use oko::iterm::api::NotificationType;
use oko::iterm::{Client, flatten, own_tty, resolve_own_session};

/// Its own name, so the dashboard's authorization is never disturbed by a diagnostic run.
const ADVISORY_NAME: &str = "oko-probe";

/// The `//!` block's three lines. `var` is deliberately absent: it is OQ-5's spike, kept as
/// a diagnostic, and not one of the things a person reaching for this binary wants.
const USAGE: &str = "\
oko-probe — Oko's headless diagnostic: what iTerm2 thinks, without a full-screen TUI in the
way. When a dashboard row looks wrong, this is what says whether iTerm2 ever reported it.

usage:
  oko-probe                      identity, then the sessions of this window
  oko-probe activate <session>   focus that session, its tab, and its window
  oko-probe watch                print notifications as they arrive
  oko-probe --help, -h           print this

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
                "unknown command {other:?}; expected `activate <session-id>`, `watch` or `var`"
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
