//! `--follow`: the same rows, as a stream another program can draw.
//!
//! Newline-delimited JSON on stdout — a header line naming the schema, then one line per
//! snapshot. Nothing here touches the terminal: no alternate screen, no key handling, no
//! footer.
//!
//! **A stream rather than a library** (§2.13). The first consumer is panex-tui, and both
//! programs are Rust, so it could depend on this crate and call [`Watcher::connect`] itself.
//! That is rejected because the feature must degrade to *nothing*: a person running panex-tui
//! without Oko installed should see no card view and no error, and a compile-time dependency
//! is always present, so there would be no absence to degrade to. The whole contract is two
//! facts — that a binary named `oko` may be on `PATH`, and the shape of the lines it writes.
//!
//! The emission point is the one that already exists. `src/iterm/watch.rs:emit_if_changed` is
//! the single place a change is published, and this mode supplies a different closure to
//! `src/iterm/watch.rs:Watcher::run` rather than adding a second view-building path. Nothing
//! here modifies that function, `src/ui.rs`, or the dashboard's path through `src/main.rs`.

use std::io::{self, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::json;

use oko::iterm::{Cmd, Event, Row, Snapshot, Watcher};
use oko::status::Age;

/// The schema a consumer must recognise, carried once per stream (OQ-9).
///
/// **Per stream and not per line**: a stream is one process and one build, so the schema
/// cannot change inside one, and a per-line marker would pay bytes forever to answer a
/// question settled at connect. Upgrading Oko does not change what a running one speaks; the
/// next launch presents a new header.
const SCHEMA: u32 = 1;

/// How often a bare newline goes out, and therefore how long a vanished consumer is tolerated.
///
/// The stream has a failure mode the dashboard does not — a consumer can vanish silently,
/// where a human closing the dashboard closes the process with it. [`emit`] alone cannot
/// bound it, because it is only called when something *changed* and §2.11 designs that to
/// happen a handful of times a day. The cost is stated rather than hidden: a consumer is woken
/// twelve times a minute, forever, by a program built to sit in a tab all day. That is the
/// price of bounding the orphan, and it is paid in one byte.
///
/// [`emit`]: Watcher::run
const KEEPALIVE: Duration = Duration::from_secs(5);

/// Connects, writes the state, and then writes every change until one end or the other goes.
pub fn run(advisory_name: &str) -> Result<()> {
    // Connect **before** anything reaches stdout: "the API is off" is a message for a human,
    // and a stream whose header had already gone out would be promising a protocol it cannot
    // speak. Stdout stays empty on this failure and the message goes to stderr, via `main`.
    let watcher = Watcher::connect(advisory_name)?;

    let mut stream = Stream::new(io::stdout());
    stream.header()?;
    // **The opening snapshot has no path through `emit_if_changed`**: `Watcher::connect` ends
    // by setting its own `emitted`, so the state at connect can never publish as a difference
    // — the dashboard gets it out of band, and so must this. Without it a panex-tui that
    // spawns Oko behind a shortcut draws an empty card view until something happens to move.
    stream.snapshot(&watcher.snapshot())?;

    thread::spawn(keepalive);

    // Nothing here sends a command, but `Watcher::run` returns immediately on a disconnected
    // channel and there is no UI holding the other end, so this sender must outlive it.
    let (_commands, commands_rx) = mpsc::channel::<Cmd>();

    let mut failure = None;
    watcher.run(&commands_rx, |event| match event {
        // A failed write is how a gone consumer is noticed on this path: Rust ignores SIGPIPE,
        // so a closed pipe is an error return rather than a signal. Stopping is the answer.
        Event::Snapshot(snapshot) => stream.snapshot(&snapshot).is_ok(),
        // A socket that dies mid-stream. Recorded rather than printed here, so that `main`
        // writes it to **stderr** exactly once and exits non-zero: stdout carries the header,
        // snapshots and keepalives, and nothing else, ever.
        Event::Error(message) => {
            failure = Some(message);
            false
        }
    });

    match failure {
        Some(message) => bail!("{message}"),
        None => Ok(()),
    }
}

/// One bare newline every [`KEEPALIVE`], and an exit when nobody is there to read it.
///
/// So blunt an exit is correct *here specifically*, for two reasons that hold in this mode and
/// not in the dashboard: this mode owns no alternate screen, so there is no terminal state a
/// teardown would have to restore — `ratatui::restore` is on the dashboard path only — and
/// `src/main.rs:run` already spawns the watcher with `thread::spawn` and never joins it, so an
/// abrupt end is what this program does anyway.
///
/// **Deliberately local to this mode.** A tick routed through `src/iterm/watch.rs:Event` would
/// reach `src/ui.rs:run`, whose `terminal.draw` sits *outside* the action match, and the
/// dashboard would redraw ten times a second forever — §2.11's stated defect at ten times the
/// rate that section rejects. Nothing would have caught it either: `log_emit` sits after
/// `emit_if_changed`'s early return, so the emission count would have looked right.
fn keepalive() {
    loop {
        thread::sleep(KEEPALIVE);
        if writeln!(io::stdout()).is_err() {
            std::process::exit(0);
        }
    }
}

/// The writing half: a header, then one line per snapshot, and never the same line twice.
///
/// **It holds `Stdout` — the handle — and never a `StdoutLock`**, and that is not an
/// oversight. Two threads write this one stdout, and `writeln!` takes the internal lock per
/// call, which is what keeps a JSON line and a keepalive from interleaving. Hoisting
/// `stdout().lock()` out of the writing loop is an ordinary-looking optimisation that starves
/// the keepalive thread forever and silently restores the orphan it exists to prevent.
struct Stream<W: Write> {
    out: W,
    /// The last line written, so one that says exactly what the last one said is dropped.
    last: Option<String>,
}

impl<W: Write> Stream<W> {
    fn new(out: W) -> Stream<W> {
        Stream { out, last: None }
    }

    /// The first line of every stream: what is speaking, and which schema it speaks.
    ///
    /// A consumer meeting a `schema` it does not know shows nothing rather than a partial row
    /// — §2.7's principle one layer out. Absence is visible; a confidently wrong card is not.
    fn header(&mut self) -> io::Result<()> {
        writeln!(self.out, "{}", header_line())
    }

    /// One snapshot, unless it serializes to the line already sent (OQ-7).
    ///
    /// The suppression is the writer's job rather than the reader's, and it is not
    /// hypothetical: `Snapshot` equality compares `Row.process`, which a row carrying a status
    /// never draws, so an iTerm2 `jobName` re-sample emits a snapshot that this schema
    /// serializes identically. Anything run in a watched pane moves that pane's deepest
    /// foreground job — a shell loop calling `sleep` once a second produced some sixty
    /// emissions a minute with nothing changing on screen.
    fn snapshot(&mut self, snapshot: &Snapshot) -> io::Result<()> {
        let line = snapshot_line(snapshot);
        if self.last.as_deref() == Some(line.as_str()) {
            return Ok(());
        }
        writeln!(self.out, "{line}")?;
        self.last = Some(line);
        Ok(())
    }
}

fn header_line() -> String {
    json!({ "oko": env!("CARGO_PKG_VERSION"), "schema": SCHEMA }).to_string()
}

/// One snapshot as one line.
///
/// `serde_json::json!` builds it, exactly as `src/status.rs:Entry::to_json` does. `Cargo.toml`
/// has `serde_json` and no `serde`, and `Row`/`Snapshot` live in the library while this mode
/// is binary-local, so reaching for `derive` would add a dependency *and* put derives in
/// `oko::iterm` for one consumer's benefit.
fn snapshot_line(snapshot: &Snapshot) -> String {
    let rows: Vec<serde_json::Value> = snapshot.rows.iter().map(row_json).collect();
    json!({ "window_number": snapshot.window_number, "rows": rows }).to_string()
}

/// One row, as the stream publishes it.
///
/// Three things it deliberately does not do. **`age` is the bucket, never seconds** — seconds
/// would make every line differ every second and destroy §2.11's quietness at the interface.
/// **`status` is the effective one**, `stale` included, because that value is derived at read
/// time and a consumer given the written one would have to re-implement two clocks to get it
/// right. And `name` and `path` are what Oko *knows*: no `-` placeholder, no `~`, no
/// truncation to a column width — those are the table's decorations, not the interface's.
///
/// **A row carrying a status carries `claude: true` and no `job`** (OQ-7, sharpened by OQ-12).
/// That `jobName` is never displayed, is ruled inadmissible as identity by OQ-2, and on a
/// Claude pane **never moves**: Claude Code spawns its tools without handing them the tty's
/// foreground process group, so the value stays the agent process for the whole session.
/// Publishing it would repeat one constant forever — it can never name the work. (The
/// "unstable within a single session" reading this comment used to carry was measured on a
/// *plain* pane, where the deepest job does churn; OQ-7 borrowed it onto Claude rows, where it
/// does not hold.) A row *without* a status carries `job` verbatim, 16-byte truncation and
/// all, because there it is the value and the only one.
fn row_json(row: &Row) -> serde_json::Value {
    let mut value = json!({
        "session_id": row.session_id,
        "tab": row.tab,
        "name": row.name,
        "path": row.path,
        "status": row.status.map(|shown| shown.status.word()),
        "age": row.status.and_then(|shown| shown.age).map(Age::label),
    });
    match row.status {
        Some(_) => value["claude"] = json!(true),
        None => value["job"] = json!(row.process),
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use oko::status::{Shown, Status};

    /// Serialization is a pure function from a `Snapshot` to a line, which is the whole reason
    /// these tests exist without an iTerm2 anywhere near them (OQ-8): the interface risk lives
    /// here, and the live half — connect, subscribe, exit — is what the exit gate is for.
    fn line_of(rows: Vec<Row>) -> serde_json::Value {
        let text = snapshot_line(&Snapshot { window_number: Some(0), rows });
        serde_json::from_str(&text).expect("every line the stream writes is JSON")
    }

    fn claude_row(shown: Shown) -> Row {
        Row {
            session_id: "F79BC39C".to_string(),
            tab: 1,
            // The value the schema must not publish for this row.
            process: Some("node".to_string()),
            path: Some("/Users/me/dev/main/oko".to_string()),
            stored_name: Some("api work".to_string()),
            name: Some("api work".to_string()),
            status: Some(shown),
        }
    }

    fn plain_row(job: Option<&str>) -> Row {
        Row {
            session_id: "B217FAEE".to_string(),
            tab: 2,
            process: job.map(str::to_owned),
            path: Some("/Users/me/dev/main".to_string()),
            stored_name: None,
            name: Some("main".to_string()),
            status: None,
        }
    }

    #[test]
    fn the_header_names_the_build_and_the_schema() {
        let header: serde_json::Value = serde_json::from_str(&header_line()).unwrap();
        assert_eq!(header["oko"], env!("CARGO_PKG_VERSION"));
        assert_eq!(header["schema"], 1);
        // One line per stream, so a consumer decides once whether it can draw this at all.
        assert!(!header_line().contains('\n'));
    }

    #[test]
    fn a_row_with_a_status_carries_claude_and_no_job() {
        let value = line_of(vec![claude_row(Shown {
            status: Status::Waiting,
            age: Some(Age::M10),
        })]);
        let row = &value["rows"][0];

        assert_eq!(row["claude"], true);
        // Not `null` — *absent*. The row's `jobName` is `node`, which is never displayed, is
        // no identity test, and moves on its own (OQ-7).
        assert!(row.get("job").is_none(), "{row}");
        assert_eq!(row["status"], "waiting");
        assert_eq!(row["age"], ">10m");
        assert_eq!(row["name"], "api work");
        assert_eq!(row["tab"], 1);
        assert_eq!(value["window_number"], 0);
    }

    #[test]
    fn a_row_without_one_carries_the_job_verbatim() {
        // What iTerm2 reports for a `rust-analyzer-proc-macro-srv`: truncated to 16 bytes, and
        // the column does not repair it — so neither does the stream.
        let value = line_of(vec![plain_row(Some("rust-analyzer-pr"))]);
        let row = &value["rows"][0];

        assert_eq!(row["job"], "rust-analyzer-pr");
        assert!(row.get("claude").is_none(), "{row}");
        // Present and null, rather than absent: this row is not a Claude tab, which is a fact
        // rather than a gap.
        assert!(row["status"].is_null(), "{row}");
        assert!(row["age"].is_null(), "{row}");
        // What Oko knows, not what the table draws: no `-`, no `~`, no truncation.
        assert_eq!(row["path"], "/Users/me/dev/main");
        assert!(line_of(vec![plain_row(None)])["rows"][0]["job"].is_null());
    }

    #[test]
    fn the_age_is_a_bucket_and_never_a_second_count() {
        for (age, label) in
            [(Age::M5, ">5m"), (Age::M10, ">10m"), (Age::M30, ">30m"), (Age::H1, ">1h")]
        {
            let value = line_of(vec![claude_row(Shown { status: Status::Ready, age: Some(age) })]);
            assert_eq!(value["rows"][0]["age"], label);
        }
        // Under five minutes there is no age at all, so a row that has just changed does not
        // make every line differ.
        let fresh = line_of(vec![claude_row(Shown { status: Status::Working, age: None })]);
        assert!(fresh["rows"][0]["age"].is_null());
    }

    #[test]
    fn stale_is_a_status_the_stream_publishes() {
        // The one place this interface deliberately disagrees with the status *file*, which
        // refuses to write `stale` because nothing is ever left to write it. It is derived at
        // read time from two clocks, and a consumer handed the written value would have to
        // re-implement both to arrive here.
        let value =
            line_of(vec![claude_row(Shown { status: Status::Stale, age: Some(Age::M30) })]);
        assert_eq!(value["rows"][0]["status"], "stale");
        assert_eq!(value["rows"][0]["claude"], true);
    }

    fn written(snapshots: &[Snapshot]) -> Vec<String> {
        let mut stream = Stream::new(Vec::new());
        for snapshot in snapshots {
            stream.snapshot(snapshot).unwrap();
        }
        String::from_utf8(stream.out).unwrap().lines().map(str::to_owned).collect()
    }

    fn snapshot_of(rows: Vec<Row>) -> Snapshot {
        Snapshot { window_number: Some(0), rows }
    }

    #[test]
    fn a_line_identical_to_the_last_is_not_written() {
        let shown = Shown { status: Status::Working, age: None };
        let one = snapshot_of(vec![claude_row(shown)]);
        assert_eq!(written(&[one.clone(), one.clone(), one]).len(), 1);
    }

    #[test]
    fn a_jobname_re_sample_on_a_claude_row_says_nothing() {
        // OQ-7's measured case, exactly: `Snapshot` equality compares `Row.process`, so this
        // pair *does* reach the closure — a shell loop calling `sleep` produced some sixty
        // such emissions a minute. The schema omits the field, and the writer drops the line.
        let shown = Shown { status: Status::Working, age: None };
        let mut resampled = claude_row(shown);
        resampled.process = Some("zsh".to_string());
        let lines =
            written(&[snapshot_of(vec![claude_row(shown)]), snapshot_of(vec![resampled])]);
        assert_eq!(lines.len(), 1, "{lines:?}");

        // The control, or the test above would pass for a writer that never writes twice: a
        // rename is a difference the schema does carry.
        let mut renamed = claude_row(shown);
        renamed.name = Some("something else".to_string());
        let lines = written(&[snapshot_of(vec![claude_row(shown)]), snapshot_of(vec![renamed])]);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[1].contains("something else"));
    }

    #[test]
    fn a_stream_opens_with_a_header_and_then_the_state() {
        let mut stream = Stream::new(Vec::new());
        stream.header().unwrap();
        stream.snapshot(&snapshot_of(vec![plain_row(Some("zsh"))])).unwrap();

        let text = String::from_utf8(stream.out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], header_line());
        // Not an empty `rows`, and not nothing at all: a consumer spawning Oko behind a
        // shortcut draws the window as it stands rather than waiting for it to move.
        let opening: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(opening["rows"].as_array().unwrap().len(), 1);
    }
}
