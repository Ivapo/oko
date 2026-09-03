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

// The shared modules, declared here rather than reached through a library. There is no
// lib target: publishing one would put an internal seam on crates.io as a public API and
// hand the vendored schema's GPL-2.0 obligation to anyone writing one dependency line
// (§2.16, OQ-13). `src/ui.rs` and `src/follow.rs` use both of them.
mod iterm;
mod status;

mod follow;
mod ui;

use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use ratatui::crossterm::event;

use iterm::{Cmd, Watcher};
use ui::AppEvent;

/// The name iTerm2 shows in its API console and keeps in its permissions list.
const ADVISORY_NAME: &str = "oko";

/// Every flag Oko answers to, in one string literal.
///
/// A literal rather than a parser: `clap` would bring a derive macro, a builder API and a
/// help format Oko does not control, to serve six flags whose entire grammar is "a flag,
/// and sometimes one or two operands" (§2.15). The cost of that choice is that this text
/// and `README.md` can disagree, so the gate checks them against each other in both
/// directions rather than trusting either.
const USAGE: &str = "\
oko — a dashboard tab inside iTerm2 showing what every other tab in the window is doing.

usage:
  oko                                  draw the dashboard for this window
  oko --follow                         newline-delimited JSON on stdout, no terminal
  oko --activate <session-id>          jump focus to that session, its tab and its window
  oko --set-name <session-id> [name]   name that session; no name clears it
  oko --version, -V                    print the version and exit, touching nothing
  oko --licenses                       print this crate's licence and its third-party notice
  oko --help, -h                       print this

Run it from a tab of the window you want watched: Oko shows the window it is itself in.
Session ids are the ones `--follow` publishes. Keys: ↑↓ select, ↵ jump, r rename, q quit.

Requires macOS and iTerm2 with the scripting API enabled
(Settings → General → Magic → Enable Python API). The status column additionally needs
Claude Code, through the hooks `oko-hook --print-settings` prints.";

/// What a person who typed `cargo install oko-iterm2` has no other way to learn.
///
/// `LICENSE` and `proto/NOTICE.md` are both in the tarball and neither is anywhere they
/// will look — `cargo` unpacks to a registry cache and puts a binary on `PATH`. It is
/// deliberately **not** a dependency-licence dump: `cargo install` builds from source and
/// the manifest already names every dependency, so forty transitive crates would bury the
/// one fact that is genuinely surprising (§2.15).
///
/// The five `api.proto` facts here are a copy of `proto/NOTICE.md`, and copies drift —
/// which is why the gate diffs them by eye rather than trusting this.
const LICENSES: &str = "\
oko-iterm2 — Oko's own source is MIT.

    Copyright (c) 2026 Ivapo
    https://github.com/Ivapo/oko/blob/main/LICENSE

One file in this crate is not Oko's work and is not covered by that licence:

    proto/api.proto
    Project   gnachman/iTerm2 — https://github.com/gnachman/iTerm2
    Commit    f4ca0004
    sha256    6f1a4e753e9c150d29454e9f83dfe91cc8d49465dde5f5aa3bda75cbd4482e31
    Licence   GPL-2.0, as iTerm2 is licensed

It is vendored verbatim as the interface definition for iTerm2's scripting API — the wire
format a client must speak to talk to iTerm2 at all — and is compiled at build time. So the
crate declares `MIT AND GPL-2.0`: that is what this artifact contains, not a choice between
the two. Taking a *library* dependency on this crate takes the GPL-2.0 obligation with it.

Full provenance is in proto/NOTICE.md. This records provenance and is not legal advice.";

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
                // Trimmed, and an empty name clears rather than blanking — exactly what the
                // dashboard's editor does (`src/ui.rs:on_key_editing`). The two doors open
                // onto the same two things or they are not the same two things: `""` would
                // decode back through `decode_json_value` as `Some("")` and render a blank
                // name, which is the state §2.10 calls a trap and which no key can produce.
                let name = rest.next().map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
                return Ok(Some(Cmd::Rename(id.clone(), name)));
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
    //
    // `--version` stays first of the three: it is the one a *program* calls, and §2.14's
    // bounded probe is keyed to it answering ahead of everything else.
    let has = |flag: &str, short: Option<&str>| {
        std::env::args().skip(1).any(|arg| arg == flag || Some(arg.as_str()) == short)
    };
    if has("--version", Some("-V")) {
        println!("oko {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // The first thing a stranger types. Before this it built a `Watcher`, opened the
    // alternate screen and drew the dashboard — the worst first contact this tool could
    // arrange, and the same thing `oko --hlep` got.
    if has("--help", Some("-h")) {
        println!("{USAGE}");
        return Ok(());
    }

    // `cargo` unpacks a crate to a registry cache and puts a binary on `PATH`, so `LICENSE`
    // and `proto/NOTICE.md` ship in the tarball and land nowhere a person will ever look.
    // This is the whole path by which an installed Oko can answer "what did I just install".
    if has("--licenses", None) {
        println!("{LICENSES}");
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

    // Nothing above recognised anything and the first argument is a flag. Before this
    // phase that drew a dashboard, which on a pipe is a panic inside `ratatui::init()`.
    //
    // **Exit 2 is taken here rather than returned.** `main` maps every `Err` to
    // `oko: {e}` and exit 1, so a refusal coming back as an error would be exit 1 wearing
    // a prefix that belongs to a failure rather than to a usage line.
    //
    // It fires only when *nothing* was recognised: `run` scans the whole argv and so does
    // `parse_command`, so `oko --hlep --version` still prints a version and a non-flag
    // operand still draws the dashboard. That is the shape Phase 6 shipped, narrowed
    // neither way.
    if args.first().is_some_and(|arg| arg.starts_with('-')) {
        eprintln!("{USAGE}");
        std::process::exit(2);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Cmd>> {
        parse_command(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    /// `Cmd` derives only `Clone` and `Debug`, and adding `PartialEq` to the library for a
    /// binary's tests is the kind of thing `--follow` already declined to do for `serde`.
    fn renamed(args: &[&str]) -> Option<String> {
        match parse(args).expect("a well-formed rename parses") {
            Some(Cmd::Rename(_, name)) => name,
            other => panic!("expected a rename, got {other:?}"),
        }
    }

    #[test]
    fn activate_takes_the_session_id_that_follows_it() {
        let Some(Cmd::Activate(id)) = parse(&["--activate", "F79BC39C"]).unwrap() else {
            panic!("expected an activate");
        };
        assert_eq!(id, "F79BC39C");
    }

    #[test]
    fn set_name_takes_a_name_and_trims_it() {
        assert_eq!(renamed(&["--set-name", "F79BC39C", "api work"]), Some("api work".into()));
        assert_eq!(renamed(&["--set-name", "F79BC39C", "  api work  "]), Some("api work".into()));
    }

    #[test]
    fn an_absent_or_empty_name_clears() {
        // The three spellings a caller might reach for, and all of them mean the same thing
        // the dashboard's editor means by an empty buffer (`src/ui.rs:on_key_editing`). `""`
        // must not survive as a name: it would render blank and no rename could escape it.
        assert_eq!(renamed(&["--set-name", "F79BC39C"]), None);
        assert_eq!(renamed(&["--set-name", "F79BC39C", ""]), None);
        assert_eq!(renamed(&["--set-name", "F79BC39C", "   "]), None);
    }

    #[test]
    fn a_flag_with_no_session_id_is_an_error_rather_than_a_dashboard() {
        // Falling through to `run`'s dashboard would put ratatui on whatever stdout is, which
        // is the failure `--version` exists to keep a consumer out of.
        assert!(parse(&["--activate"]).is_err());
        assert!(parse(&["--set-name"]).is_err());
    }

    #[test]
    fn nothing_recognised_is_not_a_command() {
        assert!(parse(&[]).unwrap().is_none());
        assert!(parse(&["--follow"]).unwrap().is_none());
    }
}
