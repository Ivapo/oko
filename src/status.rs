//! The status file: what a Claude Code tab reports, and how it gets from a hook to a row.
//!
//! One file per iTerm2 session at `~/.oko/status/<iterm-uuid>.json`, written by
//! `src/bin/oko-hook.rs` and read by `src/iterm/watch.rs`. The coupling is one directory of
//! small files, in one direction: Claude Code decides when a hook runs and runs it, and
//! never knows Oko exists.
//!
//! **A session is a Claude tab iff this directory holds a file for its iTerm2 session id**
//! (OQ-2). Staleness is a property of the *status value*, never of that identity — a stale
//! row is still a Claude row, still labelled `claude`. What keeps identity honest is
//! deletion: `SessionEnd` removes the file, and [`Store::sweep`] removes one whose pane
//! died without a hook running at all.
//!
//! Ageing is two clocks and one ladder. [`Entry::tool`] chooses which clock a `working`
//! expires on, and [`Age`] is what every status wears to say how long it has been true.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How long a `working` may go unrefreshed before it stops claiming to be true.
///
/// A trade rather than a safe number: `PreToolUse` and `PostToolUse` stamp either side of
/// every tool call, so an agent doing things stays fresh. A quiet fifteen-minute build used
/// to go stale mid-work; [`DEFAULT_TOOL_STALE_AFTER`] is what stopped it.
const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(10 * 60);

/// The same, for a `working` whose hook recorded a tool still in flight (§2.12).
///
/// **45 minutes, and the odd number is the point** (OQ-6). It must exceed
/// [`DEFAULT_STALE_AFTER`] or the mechanism does nothing; it must exceed the builds and test
/// suites the mechanism exists for; it must stay far below a working day, because an agent
/// killed mid-tool claims `working` until it expires. And it must **not** sit on a rung of
/// [`Age`]'s ladder: at 30 minutes or 1 hour the bucket fires at the same instant staleness
/// does and that rung can never render. Off the ladder, a long build legibly climbs
/// `>5m` → `>10m` → `>30m` before Oko gives up on it.
const DEFAULT_TOOL_STALE_AFTER: Duration = Duration::from_secs(45 * 60);

/// What a row says about a Claude Code session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The agent is doing something.
    Working,
    /// The agent is blocked on a human. **Never goes stale** — §1's own example is an agent
    /// that has been waiting twenty minutes, so ageing it out would delete the answer.
    Waiting,
    /// The turn is over; ready for the next prompt. Legitimately hours old.
    Ready,
    /// A `working` that stopped being refreshed. **Derived at read time, never written** —
    /// a user interrupt fires no hook at all, so nothing is left to write it.
    Stale,
}

impl Status {
    /// The word the table shows.
    pub fn word(self) -> &'static str {
        match self {
            Status::Working => "working",
            Status::Waiting => "waiting",
            Status::Ready => "ready",
            Status::Stale => "stale",
        }
    }

    /// The glyph beside it, as §1 draws them.
    pub fn glyph(self) -> &'static str {
        match self {
            Status::Working => "◐",
            Status::Waiting => "●",
            Status::Ready => "○",
            Status::Stale => "◌",
        }
    }

    /// The three a hook may write. `Stale` is deliberately absent.
    fn as_written(self) -> Option<&'static str> {
        match self {
            Status::Working | Status::Waiting | Status::Ready => Some(self.word()),
            Status::Stale => None,
        }
    }

    fn parse(word: &str) -> Option<Status> {
        match word {
            "working" => Some(Status::Working),
            "waiting" => Some(Status::Waiting),
            "ready" => Some(Status::Ready),
            _ => None,
        }
    }
}

/// How long a status has been saying what it says, in buckets (§2.11).
///
/// **One clock with one meaning**: time since the last hook fired, which is the same
/// subtraction staleness makes. Read across the four statuses it stays coherent — how long
/// you have been blocking it, how long since it last did anything, how long you have been
/// ignoring it, how long since the last credible signal.
///
/// **Buckets rather than seconds, and that is a requirement rather than a preference.** A
/// live counter would redraw the table every second, forever, for a program whose whole
/// purpose is to sit in a tab all day. A bucket changes a handful of times a day, so
/// `src/iterm/watch.rs:emit_if_changed` fires exactly at a boundary and is silent otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Age {
    M5,
    M10,
    M30,
    H1,
}

impl Age {
    /// The rung a duration falls on, or `None` under five minutes.
    ///
    /// Nothing shows an age until it starts being a question, so a row that has just changed
    /// stays clean.
    pub fn bucket(age: Duration) -> Option<Age> {
        match age.as_secs() {
            ..300 => None,
            300..600 => Some(Age::M5),
            600..1800 => Some(Age::M10),
            1800..3600 => Some(Age::M30),
            _ => Some(Age::H1),
        }
    }

    /// What the status cell shows after the word.
    pub fn label(self) -> &'static str {
        match self {
            Age::M5 => ">5m",
            Age::M10 => ">10m",
            Age::M30 => ">30m",
            Age::H1 => ">1h",
        }
    }
}

/// A status as a row draws it: the value, and how long it has said so.
///
/// One `Copy` value rather than two, because the two are read together everywhere and
/// travel together through `Row` — where `Snapshot` equality decides every redraw, so a row
/// crossing a bucket boundary has to be visible to that comparison or it never reaches the
/// screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub status: Status,
    pub age: Option<Age>,
}

/// One status file, as it sits on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The join key (§2.4): the UUID after the colon in `TERM_SESSION_ID`, which is also
    /// the session's `id` variable and `ListSessions`' `unique_identifier`.
    pub iterm_session_id: String,
    /// Claude Code's own session id. Not the join key — it is what lets one pane's
    /// successive sessions be told apart, which is what makes a conditional delete possible.
    pub claude_session_id: String,
    pub status: Status,
    pub at: OffsetDateTime,
    /// The tool this session started and has not finished, if any (§2.12).
    ///
    /// **`PreToolUse` sets it and every other event clears it.** That rule is total over
    /// §2.3's table by construction — [`write`] builds a whole `Entry` and renames it over
    /// the file, so there is no "leave this field alone" and every event that writes must
    /// decide it — and it stays total when a row is added to that table.
    ///
    /// It is only ever consulted on `working`, which is what makes so simple a rule safe:
    /// `waiting` and `ready` never age, so clearing a tool that is genuinely still running
    /// costs a row nothing.
    pub tool: Option<String>,
}

impl Entry {
    /// The status a row shows and how long it has said so.
    ///
    /// The age is `now - at` on all four statuses — one clock, one meaning (§2.11). Only a
    /// `working` can age *out*, and which threshold it ages against is the one thing [`tool`]
    /// decides: a tool in flight is not silence, so it gets the longer clock.
    ///
    /// [`tool`]: Entry::tool
    pub fn shown(
        &self,
        now: OffsetDateTime,
        stale_after: Duration,
        tool_stale_after: Duration,
    ) -> Shown {
        // Negative, and so from a clock that moved: treat it as fresh rather than as
        // impossibly old, because the next hook firing corrects it either way.
        let secs = u64::try_from((now - self.at).whole_seconds());
        let age = secs.map_or(Duration::ZERO, Duration::from_secs);
        let expired = if self.tool.is_some() { tool_stale_after } else { stale_after };
        let status = if self.status == Status::Working && age >= expired {
            Status::Stale
        } else {
            self.status
        };
        Shown { status, age: Age::bucket(age) }
    }

    fn to_json(&self) -> Result<String> {
        let status = self
            .status
            .as_written()
            .ok_or_else(|| anyhow!("{} is derived and is never written", self.status.word()))?;
        let mut value = serde_json::json!({
            "iterm_session_id": self.iterm_session_id,
            "claude_session_id": self.claude_session_id,
            "status": status,
            "at": self.at.format(&Rfc3339)?,
        });
        // Absent rather than null when there is no tool: a reader that predates this field
        // sees the file it has always seen, and the common line stays short.
        if let Some(tool) = &self.tool {
            value["tool"] = serde_json::Value::String(tool.clone());
        }
        Ok(value.to_string())
    }

    fn from_json(text: &str) -> Result<Entry> {
        let v: serde_json::Value = serde_json::from_str(text)?;
        let field = |name: &str| -> Result<String> {
            v.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("no {name}"))
        };
        let status_word = field("status")?;
        Ok(Entry {
            iterm_session_id: field("iterm_session_id")?,
            claude_session_id: field("claude_session_id")?,
            status: Status::parse(&status_word)
                .ok_or_else(|| anyhow!("unknown status {status_word:?}"))?,
            at: OffsetDateTime::parse(&field("at")?, &Rfc3339)?,
            // Optional, deliberately: every file Phase 3 wrote lacks this key, and `field`
            // above is required-only. A missing tool is "no tool", not a corrupt file.
            tool: v.get("tool").and_then(serde_json::Value::as_str).map(str::to_owned),
        })
    }
}

/// `~/.oko`, absolute — everything Oko writes lives under it.
///
/// Absolute because a hook runs with `cwd` set to whichever project *that* session is in,
/// so nothing relative and nothing under `$CLAUDE_PROJECT_DIR` would resolve here.
pub fn oko_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME is unset, so there is nowhere to write")?;
    if home.is_empty() {
        bail!("HOME is empty, so there is nowhere to write");
    }
    Ok(PathBuf::from(home).join(".oko"))
}

/// `~/.oko/status`, the one directory the hook writes and the dashboard reads.
pub fn status_dir() -> Result<PathBuf> {
    Ok(oko_dir()?.join("status"))
}

fn file_of(dir: &Path, iterm_session_id: &str) -> PathBuf {
    dir.join(format!("{iterm_session_id}.json"))
}

/// Writes one status file, atomically.
///
/// Temp-file-plus-rename **in the same directory**, because Oko reads it concurrently and a
/// half-written file is a row that lies. The rename is also what moves the directory's
/// mtime, which is what [`Store::refresh`] watches.
pub fn write(entry: &Entry) -> Result<()> {
    let dir = status_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let temp = dir.join(format!(".{}.json.{}.tmp", entry.iterm_session_id, std::process::id()));
    fs::write(&temp, entry.to_json()?).with_context(|| format!("writing {}", temp.display()))?;
    match fs::rename(&temp, file_of(&dir, &entry.iterm_session_id)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&temp);
            Err(e).context("renaming the status file into place")
        }
    }
}

/// Deletes a session's status file, but **only if it belongs to the Claude session asking**.
///
/// `SessionEnd` and `SessionStart` both fire on `/clear`, in an order nothing documents. An
/// unconditional delete would let the ending session erase the successor's fresh `ready`
/// and leave the row blank until the next event. Comparing the id makes that impossible.
pub fn remove_if_owned(iterm_session_id: &str, claude_session_id: &str) -> Result<()> {
    let path = file_of(&status_dir()?, iterm_session_id);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        // Already gone is the outcome we wanted.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("reading the status file before deleting it"),
    };
    if Entry::from_json(&text).is_ok_and(|e| e.claude_session_id == claude_session_id) {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}

/// How long a `working` may go unrefreshed, from `OKO_STALE_AFTER`.
///
/// Accepts bare seconds or a `s`/`m`/`h` suffix. An unreadable value falls back to the
/// default rather than failing: this is read on the way into a TUI, and a dashboard that
/// refuses to start over a malformed environment variable is worse than one that is
/// conservative about ageing.
pub fn stale_after() -> Duration {
    from_env("OKO_STALE_AFTER", DEFAULT_STALE_AFTER)
}

/// The same, for a `working` carrying a tool in flight, from `OKO_TOOL_STALE_AFTER`.
///
/// **A longer clock, not an exemption** (§2.12): if an agent is killed mid-tool no
/// `PostToolUse` ever arrives, and an unbounded exemption would leave that row claiming
/// `working` forever — §2.7's confidently-wrong failure, slowed down.
pub fn tool_stale_after() -> Duration {
    from_env("OKO_TOOL_STALE_AFTER", DEFAULT_TOOL_STALE_AFTER)
}

fn from_env(name: &str, default: Duration) -> Duration {
    std::env::var(name).ok().and_then(|raw| parse_duration(&raw)).unwrap_or(default)
}

fn parse_duration(raw: &str) -> Option<Duration> {
    let raw = raw.trim();
    let (digits, scale) = match raw.strip_suffix(['s', 'S']) {
        Some(d) => (d, 1),
        None => match raw.strip_suffix(['m', 'M']) {
            Some(d) => (d, 60),
            None => match raw.strip_suffix(['h', 'H']) {
                Some(d) => (d, 3600),
                None => (raw, 1),
            },
        },
    };
    digits.trim().parse::<u64>().ok().map(|n| Duration::from_secs(n * scale))
}

/// Every status file, and the mtime that says whether they need re-reading.
///
/// Keyed by iTerm2 session id, so merging into a row is an exact UUID match and there is no
/// name matching anywhere.
pub struct Store {
    dir: PathBuf,
    /// The directory's mtime when [`refresh`](Self::refresh) last re-read it. `None` until
    /// the first successful read, so a directory that does not exist yet is retried.
    mtime: Option<SystemTime>,
    entries: HashMap<String, Entry>,
    stale_after: Duration,
    tool_stale_after: Duration,
}

impl Store {
    pub fn open() -> Store {
        Store::in_dir(status_dir().unwrap_or_default(), stale_after(), tool_stale_after())
    }

    /// The same store over a named directory, so the mtime gate and the sweep can be driven
    /// without a `$HOME` to point at.
    pub fn in_dir(dir: PathBuf, stale_after: Duration, tool_stale_after: Duration) -> Store {
        Store { dir, mtime: None, entries: HashMap::new(), stale_after, tool_stale_after }
    }

    /// Re-reads the directory, but only when its mtime moved.
    ///
    /// **Not the polling OQ-3 ruled out** — that answer is about iTerm2's API, which still
    /// pushes. This is one `stat` against a local directory on a tick the watcher already
    /// wakes on, and a filesystem-notification crate would be a dependency and a second
    /// event source for no gain a stopwatch can see.
    pub fn refresh(&mut self) {
        let Ok(mtime) = fs::metadata(&self.dir).and_then(|m| m.modified()) else {
            // No directory yet: no hook has ever run. Keep nothing rather than keeping stale
            // entries, so a deleted directory empties the column.
            if self.mtime.take().is_some() {
                self.entries.clear();
            }
            return;
        };
        if self.mtime == Some(mtime) {
            return;
        }

        let Ok(dir) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut entries = HashMap::new();
        for path in dir.flatten().map(|e| e.path()) {
            // Skip the temp files a concurrent write leaves in this same directory.
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path)
                && let Ok(entry) = Entry::from_json(&text)
            {
                entries.insert(entry.iterm_session_id.clone(), entry);
            }
        }
        self.mtime = Some(mtime);
        self.entries = entries;
    }

    /// The status to show for a session, `None` when it is not a Claude tab.
    pub fn status_of(&self, iterm_session_id: &str, now: OffsetDateTime) -> Option<Shown> {
        self.entries
            .get(iterm_session_id)
            .map(|e| e.shown(now, self.stale_after, self.tool_stale_after))
    }

    /// Deletes every status file whose session is not in `live`.
    ///
    /// `live` must be **every session iTerm2 knows about**, not the rows of one window:
    /// rows are window-scoped, so sweeping against them would destroy the live status of
    /// Claude tabs in other windows, and two Okos in two windows would delete each other's
    /// files continuously. This is what covers a `kill -9`, where no hook runs at all.
    pub fn sweep(&mut self, live: &std::collections::HashSet<&str>) {
        let dead: Vec<String> =
            self.entries.keys().filter(|id| !live.contains(id.as_str())).cloned().collect();
        for id in dead {
            // A failed unlink is not worth killing the dashboard over; the next sweep retries.
            let _ = fs::remove_file(file_of(&self.dir, &id));
            self.entries.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_with_and_without_a_suffix() {
        assert_eq!(parse_duration("600"), Some(Duration::from_secs(600)));
        assert_eq!(parse_duration("10s"), Some(Duration::from_secs(10)));
        assert_eq!(parse_duration(" 10m "), Some(Duration::from_secs(600)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration(""), None);
    }

    fn entry(status: Status, at: OffsetDateTime) -> Entry {
        Entry {
            iterm_session_id: "F79BC39C-B1C1-47C3-9E9D-6820789978D9".to_string(),
            claude_session_id: "3f1b5d4c-0000-4000-8000-000000000000".to_string(),
            status,
            at,
            tool: None,
        }
    }

    /// The two thresholds the gate runs with, so a test reads like the runbook.
    const SHORT: Duration = Duration::from_secs(600);
    const LONG: Duration = Duration::from_secs(2700);

    fn status_at(e: &Entry, now: OffsetDateTime) -> Status {
        e.shown(now, SHORT, LONG).status
    }

    #[test]
    fn a_status_file_survives_a_round_trip() {
        let written =
            entry(Status::Waiting, OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap());
        let read = Entry::from_json(&written.to_json().unwrap()).unwrap();
        assert_eq!(read, written);
    }

    #[test]
    fn a_tool_in_flight_survives_a_round_trip() {
        let mut written =
            entry(Status::Working, OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap());
        written.tool = Some("Bash".to_string());
        let read = Entry::from_json(&written.to_json().unwrap()).unwrap();
        assert_eq!(read, written);
        assert_eq!(read.tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn a_file_written_before_the_tool_field_existed_still_reads() {
        // Byte for byte what Phase 3 wrote. `from_json`'s required-field closure would
        // reject this if `tool` went through it, and every live session would go blank.
        let phase_3 = r#"{"iterm_session_id":"AAAA","claude_session_id":"BBBB",
                          "status":"working","at":"2026-08-15T00:00:00Z"}"#;
        let read = Entry::from_json(phase_3).unwrap();
        assert_eq!(read.status, Status::Working);
        assert_eq!(read.tool, None);
    }

    #[test]
    fn only_working_goes_stale() {
        let now = OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap();
        let long_ago = now - Duration::from_secs(3600);

        assert_eq!(status_at(&entry(Status::Working, long_ago), now), Status::Stale);
        assert_eq!(status_at(&entry(Status::Working, now), now), Status::Working);
        // §1's own example is an agent that has been waiting twenty minutes.
        assert_eq!(status_at(&entry(Status::Waiting, long_ago), now), Status::Waiting);
        assert_eq!(status_at(&entry(Status::Ready, long_ago), now), Status::Ready);
    }

    #[test]
    fn a_tool_in_flight_ages_on_the_longer_clock() {
        let now = OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap();
        let quiet_build = |minutes: u64| {
            let mut e = entry(Status::Working, now - Duration::from_secs(minutes * 60));
            e.tool = Some("Bash".to_string());
            e
        };

        // The case §2.12 exists for: fifteen quiet minutes of `Bash`, which the short clock
        // would have called stale at ten.
        assert_eq!(status_at(&quiet_build(15), now), Status::Working);
        let same_age_no_tool = entry(Status::Working, now - Duration::from_secs(15 * 60));
        assert_eq!(status_at(&same_age_no_tool, now), Status::Stale);

        // A longer clock, not an exemption: an agent killed mid-tool still gives up.
        assert_eq!(status_at(&quiet_build(46), now), Status::Stale);
    }

    #[test]
    fn a_clock_that_moved_backwards_reads_as_fresh() {
        let now = OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap();
        let future = now + Duration::from_secs(3600);
        let ahead = entry(Status::Working, future);
        assert_eq!(status_at(&ahead, now), Status::Working);
        // And carries no age either, rather than an hour of it.
        assert_eq!(ahead.shown(now, SHORT, LONG).age, None);
    }

    #[test]
    fn the_ladder_has_four_rungs_and_a_quiet_bottom() {
        let at = |secs: u64| Age::bucket(Duration::from_secs(secs));
        // Nothing under five minutes: a row that just changed stays clean.
        assert_eq!(at(0), None);
        assert_eq!(at(299), None);
        assert_eq!(at(300), Some(Age::M5));
        assert_eq!(at(599), Some(Age::M5));
        assert_eq!(at(600), Some(Age::M10));
        assert_eq!(at(1799), Some(Age::M10));
        assert_eq!(at(1800), Some(Age::M30));
        assert_eq!(at(3599), Some(Age::M30));
        assert_eq!(at(3600), Some(Age::H1));
        assert_eq!(at(86_400), Some(Age::H1));

        assert_eq!(Age::M5.label(), ">5m");
        assert_eq!(Age::M10.label(), ">10m");
        assert_eq!(Age::M30.label(), ">30m");
        assert_eq!(Age::H1.label(), ">1h");
    }

    #[test]
    fn the_tool_threshold_sits_off_the_ladder() {
        // OQ-6's fourth bound, as an assertion rather than a paragraph: on any rung, that
        // rung fires at the same instant staleness does and can never render.
        for rung in [300, 600, 1800, 3600] {
            assert_ne!(DEFAULT_TOOL_STALE_AFTER.as_secs(), rung);
        }
        assert!(DEFAULT_TOOL_STALE_AFTER > DEFAULT_STALE_AFTER);
        // So the top reachable rung for a long-running tool is `>30m`, held for a quarter
        // of an hour before Oko gives up.
        assert_eq!(Age::bucket(DEFAULT_TOOL_STALE_AFTER - Duration::from_secs(1)), Some(Age::M30));
    }

    #[test]
    fn every_status_carries_an_age() {
        let now = OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap();
        let twenty_minutes = now - Duration::from_secs(20 * 60);
        // §1's motivating sentence, on screen at last.
        for status in [Status::Waiting, Status::Ready] {
            assert_eq!(
                entry(status, twenty_minutes).shown(now, SHORT, LONG),
                Shown { status, age: Some(Age::M10) }
            );
        }
        // Including the one that is derived: for `stale` the age *is* the reason.
        assert_eq!(
            entry(Status::Working, twenty_minutes).shown(now, SHORT, LONG),
            Shown { status: Status::Stale, age: Some(Age::M10) }
        );
    }

    #[test]
    fn stale_is_never_written() {
        let now = OffsetDateTime::from_unix_timestamp(1_755_216_000).unwrap();
        assert!(entry(Status::Stale, now).to_json().is_err());
    }

    /// A directory of our own, named for the test so two of them cannot collide.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oko-status-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, iterm: &str, status: Status) {
        let entry = Entry {
            iterm_session_id: iterm.to_string(),
            claude_session_id: format!("claude-{iterm}"),
            status,
            at: OffsetDateTime::now_utc(),
            tool: None,
        };
        fs::write(file_of(dir, iterm), entry.to_json().unwrap()).unwrap();
    }

    /// What the table shows, ignoring the age — the assertion most of these tests want.
    fn shown_status(store: &Store, id: &str, now: OffsetDateTime) -> Option<Status> {
        store.status_of(id, now).map(|s| s.status)
    }

    #[test]
    fn a_store_reads_what_the_hook_wrote_and_ignores_the_temp_file() {
        let dir = scratch("reads");
        put(&dir, "AAAA", Status::Waiting);
        // The shape `write` leaves mid-rename, in this same directory.
        fs::write(dir.join(".BBBB.json.999.tmp"), "half a fi").unwrap();

        let mut store = Store::in_dir(dir, SHORT, LONG);
        store.refresh();
        let now = OffsetDateTime::now_utc();
        assert_eq!(shown_status(&store, "AAAA", now), Some(Status::Waiting));
        assert_eq!(shown_status(&store, "BBBB", now), None);
        // Not a Claude tab: no file, no status, and no guessing from a process name.
        assert_eq!(shown_status(&store, "CCCC", now), None);
    }

    #[test]
    fn the_sweep_keeps_live_sessions_and_deletes_the_rest() {
        let dir = scratch("sweep");
        put(&dir, "ALIVE", Status::Working);
        put(&dir, "BURIED", Status::Waiting);
        put(&dir, "GONE", Status::Ready);

        let mut store = Store::in_dir(dir.clone(), SHORT, LONG);
        store.refresh();
        // What `rescan` passes: every session of every window, plus the buried ones.
        store.sweep(&std::collections::HashSet::from(["ALIVE", "BURIED"]));

        let now = OffsetDateTime::now_utc();
        assert_eq!(shown_status(&store, "ALIVE", now), Some(Status::Working));
        assert_eq!(shown_status(&store, "BURIED", now), Some(Status::Waiting));
        assert_eq!(shown_status(&store, "GONE", now), None);
        assert!(!file_of(&dir, "GONE").exists());
        assert!(file_of(&dir, "ALIVE").exists());
    }

    #[test]
    fn a_working_row_goes_stale_without_anything_being_written() {
        let dir = scratch("stale");
        put(&dir, "AAAA", Status::Working);

        let mut store = Store::in_dir(dir, SHORT, LONG);
        store.refresh();
        let now = OffsetDateTime::now_utc();
        assert_eq!(shown_status(&store, "AAAA", now), Some(Status::Working));
        // Nothing wrote a file and nothing fired: only the clock moved. This is the path
        // gate check 7 walks, and the reason the merged view is compared on every tick.
        let later = now + Duration::from_secs(601);
        assert_eq!(shown_status(&store, "AAAA", later), Some(Status::Stale));
        // And the bucket moved with it, which is what makes a boundary reach the screen.
        assert_eq!(store.status_of("AAAA", later).unwrap().age, Some(Age::M10));
    }
}
