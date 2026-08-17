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

use anyhow::Result;
use ratatui::crossterm::event;

use oko::iterm::Watcher;
use ui::AppEvent;

/// The name iTerm2 shows in its API console and keeps in its permissions list.
const ADVISORY_NAME: &str = "oko";

fn main() {
    if let Err(e) = run() {
        eprintln!("oko: {e:#}");
        std::process::exit(1);
    }
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
