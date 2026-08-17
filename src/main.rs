//! Oko — the dashboard tab. Phase 2 of `specs/tab_dashboard_spec.md`.
//!
//! Three threads. One owns the iTerm2 socket and every conversation with it; one blocks on
//! terminal input; the main thread draws and does nothing else. They meet on one channel,
//! so a keystroke and a notification arrive at the table the same way.
//!
//! **`--follow` is a second entry point**, taken before any of that: the same rows as a JSON
//! stream for another program to draw, with no terminal involved at all (`src/follow.rs`).
//!
//! The API must be enabled once per machine — see `README.md` and `rules/iterm-api.md`.

mod follow;
mod ui;

use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use ratatui::crossterm::event;

use oko::iterm::{Cmd, Watcher};
use ui::AppEvent;

/// The name iTerm2 shows in its API console and keeps in its permissions list.
const ADVISORY_NAME: &str = "oko";

fn main() {
    if let Err(e) = run() {
        eprintln!("oko: {e:#}");
        std::process::exit(1);
    }
}

/// `--activate <session>` jumps focus to a session; `--set-name <session>
/// [name]` names one, and clears it when the name is left off — which is the
/// only way back to the derived default, the same as in the dashboard.
///
/// Session ids are the ones `--follow` publishes, which is the whole point:
/// the stream says what a consumer keys a card on, and these take that key.
fn parse_command(args: &[String]) -> Result<Option<Cmd>> {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--activate" => {
                let id = rest.next().context("--activate needs a session id")?;
                return Ok(Some(Cmd::Activate(id.clone())));
            }
            "--set-name" => {
                let id = rest.next().context("--set-name needs a session id")?;
                return Ok(Some(Cmd::Rename(id.clone(), rest.next().cloned())));
            }
            _ => {}
        }
    }
    Ok(None)
}

fn run() -> Result<()> {
    // Ahead of everything, and cheapest of all: a consumer deciding whether this binary
    // speaks `--follow` should not have to spawn a stream and an iTerm2 connection to find
    // out. Falling through to the dashboard on an unrecognised flag makes the question
    // unanswerable — with a pipe for stdout that path panics inside `ratatui::init()`, so
    // what the caller learns is a dead child and an escape sequence.
    if std::env::args().skip(1).any(|arg| arg == "--version" || arg == "-V") {
        println!("oko {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Before `ratatui::init()`, and before the connection: the stream mode owns no terminal
    // and shares nothing with the dashboard's path but this line.
    if std::env::args().skip(1).any(|arg| arg == "--follow") {
        return follow::run(ADVISORY_NAME);
    }

    // The two things the dashboard can do, reachable without one — so a
    // program drawing the stream can act on what it drew instead of telling
    // the user to come back here and press a key.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(cmd) = parse_command(&args)? {
        let mut watcher = Watcher::connect(ADVISORY_NAME)?;
        return watcher.execute(cmd);
    }

    // Connect before the alternate screen exists: "the API is off" is a message a human
    // acts on, and it would otherwise flash past between init and restore.
    let watcher = Watcher::connect(ADVISORY_NAME)?;
    let initial = watcher.snapshot();

    let (events_tx, events_rx) = mpsc::channel();
    let (commands_tx, commands_rx) = mpsc::channel();

    let socket_events = events_tx.clone();
    thread::spawn(move || {
        watcher.run(&commands_rx, |event| socket_events.send(AppEvent::Iterm(event)).is_ok());
    });

    thread::spawn(move || {
        // Ends when the event loop drops the receiver, which is how quitting is signalled.
        while let Ok(event) = event::read() {
            if events_tx.send(AppEvent::Terminal(event)).is_err() {
                break;
            }
        }
    });

    let mut terminal = ratatui::init();
    let result = ui::run(&mut terminal, &events_rx, &commands_tx, initial);
    ratatui::restore();
    result
}
