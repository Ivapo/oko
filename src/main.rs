//! Oko — the dashboard tab. Phase 2 of `specs/tab_dashboard_spec.md`.
//!
//! Three threads. One owns the iTerm2 socket and every conversation with it; one blocks on
//! terminal input; the main thread draws and does nothing else. They meet on one channel,
//! so a keystroke and a notification arrive at the table the same way.
//!
//! The API must be enabled once per machine — see `README.md` and `rules/iterm-api.md`.

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
