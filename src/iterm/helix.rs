//! What a Helix pane has open, read off its screen (§2.17).
//!
//! A pure function and nothing else: no client, no row, no clock. It takes the rows of a
//! screen and answers one of three things, and every one of its tests is a screen captured
//! from a real Helix 25.07.1 with `oko-probe hx <session>` — a hand-typed fixture tests the
//! parser against its author's memory of the screen.
//!
//! **This is the one place in Oko that reads a user interface**, which §2.7 rejected for
//! *status* and §2.17 admits here because nothing structured carries a Helix pane's file at
//! all. The whole shape of the module is §2.7's principle kept rather than argued around:
//! **the failure mode must be a value that stops updating, not one that lies.** So a screen
//! yields a file only from a line matching the status line's shape in full, two candidate
//! lines yield nothing rather than the first, and everything else answers
//! [`Open::NoStatusLine`], which leaves the row reading plain `hx`.
//!
//! **A submodule of `iterm` rather than a root module, and that is Phase 8's table talking**:
//! declared at a binary's root it would have to be added to `oko` and `oko-probe` both, and
//! `oko-hook` — which declares `status` alone — must not reach it at all.

/// The one job name a screen is read for (§2.17). Not an editor family: `hx`, exactly.
///
/// An editor that sets a title — Neovim's `title` option — would offer a structured source,
/// which is a different decision and one nobody has asked for.
pub const JOB_NAME: &str = "hx";

/// Helix's default mode names, space-padded as `render_mode` writes them.
///
/// Five cells whichever it is, because an **unfocused** view's `render_mode` writes five
/// blanks — which is what makes a piece beginning with one of these the focused view's.
const MODES: [&str; 3] = [" NOR ", " INS ", " SEL "];

/// The mode alone, which is as far as the candidate test reads.
const MODE_CELLS: usize = 5;

/// Mode, then the cell Helix reserves for the language-server spinner **whether or not one
/// is spinning**, then `file-name`'s own leading space.
///
/// **Counted, never trimmed**, and the difference is not cosmetic: `trim_start` on a pane
/// whose language server is starting takes the spinner glyph with it and the row reads
/// `hx ⣾ main.rs`. Measured against a real status line —
/// `" NOR   src/main.rs …"` — where the path begins at exactly this offset.
const PREFIX_CELLS: usize = 7;

/// `helix-tui`'s `symbols::line::VERTICAL`, drawn at `area.right()` for **every** row of a
/// view including its status row — so "the cell after a separator" is a boundary that always
/// exists rather than one observed once.
const SEPARATOR: char = '│';

/// What Helix calls a buffer with no file. An answer, not a file name.
const SCRATCH: &str = "[scratch]";

/// Where the file name ends: the read-only indicator, the modification indicator, or the
/// first run of two spaces (§2.17).
///
/// **All three measured on this machine, 2026-09-17, Helix 25.07.1.** A modified file draws
/// `src/main.rs [+]` — one space, which only the third of these catches. A read-only file
/// draws `/etc/hosts  [readonly]` — two spaces, so the run already ends it and
/// `" [readonly]"` is belt and braces against a release that drops one of them. An ordinary
/// file is followed by the padding before the right-hand section, which is a run of two.
///
/// **A single space is deliberately not a terminator**: a path containing one is among the
/// states §2.17 measured, and cutting there would rename the file.
const TERMINATORS: [&str; 3] = ["  ", " [readonly]", " [+]"];

/// What one screenful of a Helix pane says about the file open in it.
///
/// **The third is not the second**, and conflating them is the bug this split exists to
/// prevent: `[scratch]` must clear a row's file, while an overlay must leave a correct one
/// alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// The file's base name. The status line's path is relative to Helix's working
    /// directory, which need not be the row's — so showing the path would be a second
    /// `where` rooted somewhere else, and the base name answers "which file" and no more.
    File(String),
    /// A status line that names no file.
    NoFile,
    /// No row piece matched, or more than one did.
    NoStatusLine,
}

/// The file open in the focused view of a Helix screen (§2.17).
///
/// The rule, and every clause of it is load-bearing: split each row on [`SEPARATOR`], take a
/// piece that **starts** with a space-padded mode name and carries a `line:col` later in that
/// same piece, and require **exactly one** such piece on the whole screen.
///
/// **The line is found by where it starts, not by where the cursor is.** A file's own text can
/// look exactly like a status line, and in this repository one does — §2.17 quotes a real
/// status line inside the document, so editing that document in Helix puts a counterfeit on
/// the screen. What separates them is the left edge: Helix draws buffer text after a gutter
/// and a status line at its view's own first column. Two rules that look right on paper were
/// measured and failed: anchoring on the terminal cursor takes the counterfeit whenever one
/// sits between the cursor and the real line, and anchoring on the status line's background
/// dies on this machine's own theme, under which every cell of the screen reports one colour.
///
/// **Indexing a piece from the left is the same as counting screen columns here, and only
/// here.** `LineContents.text` drops uninitialized cells, so in general a column is
/// `code_points_per_cell`'s job rather than a string offset — but Helix paints every cell of
/// every row it draws, so a Helix pane has no uninitialized cell for the text to close up,
/// and everything this reads is single-width and non-combining and sits to the *left* of any
/// path a wide character could appear in. A pane that is not true of is not a Helix pane, and
/// it fails to match rather than matching wrongly.
pub fn open_file(rows: &[String]) -> Open {
    let mut found: Option<&str> = None;
    for row in rows {
        for piece in row.split(SEPARATOR) {
            if !is_status_line(piece) {
                continue;
            }
            // **The uniqueness requirement is the counterfeit defence.** A second candidate
            // is not a tie to be broken in favour of the first: the answer is that this
            // screen does not say, which is what a gutterless pane showing a crafted line
            // produces and what §2.17 requires of it.
            if found.is_some() {
                return Open::NoStatusLine;
            }
            found = Some(piece);
        }
    }

    let Some(piece) = found else {
        return Open::NoStatusLine;
    };
    let Some(path) = path_of(piece) else {
        return Open::NoStatusLine;
    };
    if path == SCRATCH {
        return Open::NoFile;
    }
    match base_name(path) {
        Some(name) => Open::File(name.to_string()),
        None => Open::NoStatusLine,
    }
}

/// Whether this piece of a row is a focused view's status line.
fn is_status_line(piece: &str) -> bool {
    MODES.iter().any(|mode| piece.starts_with(mode)) && carries_a_position(piece)
}

/// Whether a `line:col` appears in the piece after the mode name.
///
/// Byte-indexed, and safe for it: a UTF-8 continuation byte is neither `b':'` nor an ASCII
/// digit, so a wide glyph anywhere in the piece cannot fake one.
fn carries_a_position(piece: &str) -> bool {
    let bytes = piece.as_bytes();
    if bytes.len() <= MODE_CELLS {
        return false;
    }
    bytes[MODE_CELLS..]
        .windows(3)
        .any(|w| w[1] == b':' && w[0].is_ascii_digit() && w[2].is_ascii_digit())
}

/// The path the status line names, between the reserved prefix and the first terminator.
///
/// `None` where the piece is shorter than the prefix, or where **no terminator is present at
/// all** — which is absence rather than "the rest of the piece". Taking the remainder would
/// concatenate the path with the right-hand section and put a fabricated name in the cell,
/// which is the one failure §2.17 is built to rule out.
fn path_of(piece: &str) -> Option<&str> {
    let start = piece.char_indices().nth(PREFIX_CELLS)?.0;
    let rest = &piece[start..];
    let end = TERMINATORS.iter().filter_map(|marker| rest.find(marker)).min()?;
    let path = &rest[..end];
    (!path.is_empty()).then_some(path)
}

/// The last component of a path.
///
/// Deliberately a second, smaller copy of `src/iterm/watch.rs:last_component`'s idea rather
/// than a shared helper: that one is about a `path` *variable* and answers `/` for the root,
/// which is a place and not a file name.
fn base_name(path: &str) -> Option<&str> {
    path.rsplit('/').find(|part| !part.is_empty())
}

#[cfg(test)]
mod tests {
    //! Every fixture below is a screen captured from a real Helix 25.07.1 on 2026-09-17 with
    //! `oko-probe hx <session>`, driven through the iTerm2 API in a scratch window. **Not one
    //! of them is hand-typed**, and that is the point: a hand-typed fixture tests the parser
    //! against its author's memory of the screen, and the whole risk this module carries is
    //! that the screen is not what its reader thinks.
    //!
    //! They are one screen row per line, verbatim, trailing spaces included — Helix paints
    //! every cell it draws. The trailing run is *not* load-bearing: the terminator that ends
    //! a file name sits between the path and the right-hand section, in the middle of the
    //! row, so a fixture some editor has stripped still parses to the same answer.
    //!
    //! **This is where a Helix release that reshapes the status line shows up first**, which
    //! is §2.17's answer to the version-less-UI risk §2.7 names: when one of these stops
    //! parsing, the line moved.

    use super::*;

    /// §2.17: "a single view with a counterfeit line above the cursor". This repository's own
    /// spec quotes a real status line, so editing it puts a counterfeit on screen — here at
    /// screen rows 18 and 35, with the cursor below both (the status line reads `916:16`).
    const COUNTERFEIT_ABOVE: &str = include_str!("helix_fixtures/counterfeit-above.txt");
    /// §2.17: "then below it" — the same screen with the cursor above the counterfeits
    /// (`904:16`), which is the position the rejected cursor rule got wrong.
    const COUNTERFEIT_BELOW: &str = include_str!("helix_fixtures/counterfeit-below.txt");
    /// §2.17: "then under it" — the cursor on the counterfeit's own line (`913:16`).
    const COUNTERFEIT_UNDER: &str = include_str!("helix_fixtures/counterfeit-under.txt");
    /// §2.17: "a vertical split focused either side" — focus in the left view.
    const VSPLIT_LEFT: &str = include_str!("helix_fixtures/vsplit-left.txt");
    /// …and in the right one. Both views' status lines share one screen row, separated by
    /// `│`, and the unfocused one writes blanks where the mode would be.
    const VSPLIT_RIGHT: &str = include_str!("helix_fixtures/vsplit-right.txt");
    /// §2.17: "three views".
    const THREE_VIEWS: &str = include_str!("helix_fixtures/three-views.txt");
    /// §2.17: "a path containing a space".
    const PATH_WITH_SPACE: &str = include_str!("helix_fixtures/path-with-space.txt");
    /// §2.17: "a read-only file". Helix draws `/etc/hosts  [readonly]`.
    const READ_ONLY: &str = include_str!("helix_fixtures/read-only.txt");
    /// §2.17: "`[scratch]`".
    const SCRATCH_BUFFER: &str = include_str!("helix_fixtures/scratch.txt");
    /// §2.17: "`[+]` in insert mode". Helix draws `/tmp/oko-9.txt [+]` — one space, which is
    /// why the two-space run is not the only terminator.
    const INSERT_MODIFIED: &str = include_str!("helix_fixtures/insert-modified.txt");
    /// §2.17: "an open command line with its completion menu". At this window's height the
    /// menu **covered** the status line, which is the case that makes the third answer
    /// necessary.
    const COMMAND_LINE_MENU: &str = include_str!("helix_fixtures/command-line-menu.txt");
    /// §2.17: "an open file picker". It did *not* cover the status line — and it draws its
    /// own borders out of the same `│` the rule splits on.
    const FILE_PICKER: &str = include_str!("helix_fixtures/file-picker.txt");
    /// Check 12's first extra: the gutterless screen carrying a counterfeit that **begins a
    /// row piece**, so the screen holds two candidates. Captured from the crafted
    /// `/tmp/oko-9-decoy.txt` under `gutters = []`, never from this document — a counterfeit
    /// that does not begin a row piece answers "one candidate" and pins nothing.
    const GUTTERLESS_COUNTERFEIT: &str = include_str!("helix_fixtures/gutterless-counterfeit.txt");
    /// Check 12's second extra: an empty screen. A cleared pane, 40 blank rows.
    const EMPTY_SCREEN: &str = include_str!("helix_fixtures/empty-screen.txt");
    /// **Beyond §2.17's enumeration, and deliberately**: check 5's own condition as a unit
    /// test. `[editor.statusline] left = ["file-name"], right = []` leaves Helix no mode and
    /// no position, and §2.7's requirement is that such a line produce *nothing*.
    const CUSTOM_STATUSLINE: &str = include_str!("helix_fixtures/custom-statusline.txt");
    /// **Also beyond the enumeration**, and the only fixture that exercises [`PREFIX_CELLS`]
    /// against a spinner: captured while rust-analyzer started, the line reads
    /// `" NOR ⣾ src/iterm/watch.rs"`. An implementation that trimmed the prefix instead of
    /// counting it passes every other fixture here and answers `⣾ watch.rs` for this one.
    const SPINNER: &str = include_str!("helix_fixtures/spinner.txt");

    fn read(fixture: &str) -> Open {
        let rows: Vec<String> = fixture.lines().map(str::to_string).collect();
        open_file(&rows)
    }

    fn file(name: &str) -> Open {
        Open::File(name.to_string())
    }

    #[test]
    fn a_counterfeit_in_the_buffer_never_becomes_the_answer() {
        // All three read *this document*, never `main.py` — the file the counterfeit names.
        // **These three are one regression test against both rules §2.17 rejected**, and the
        // reason they are three: the cursor-anchored rule answers differently in each.
        for fixture in [COUNTERFEIT_ABOVE, COUNTERFEIT_BELOW, COUNTERFEIT_UNDER] {
            assert_eq!(read(fixture), file("tab_dashboard_spec.md"));
        }
    }

    #[test]
    fn a_split_names_the_file_of_the_focused_view() {
        // Not the bottom line on the screen: both views write a status line into the same
        // row, and only the focused one begins with a mode name.
        assert_eq!(read(VSPLIT_LEFT), file("main.rs"));
        assert_eq!(read(VSPLIT_RIGHT), file("follow.rs"));
        assert_eq!(read(THREE_VIEWS), file("ui.rs"));
    }

    #[test]
    fn a_path_keeps_the_spaces_inside_it() {
        // A single space cannot end a file name; a run of two can.
        assert_eq!(read(PATH_WITH_SPACE), file("oko-9 space.txt"));
    }

    #[test]
    fn an_indicator_is_not_part_of_the_name() {
        assert_eq!(read(READ_ONLY), file("hosts"));
        assert_eq!(read(INSERT_MODIFIED), file("oko-9.txt"));
    }

    #[test]
    fn a_buffer_with_no_file_is_not_a_file() {
        // `NoFile`, which clears a row's name — and is emphatically not `NoStatusLine`,
        // which leaves it alone.
        assert_eq!(read(SCRATCH_BUFFER), Open::NoFile);
    }

    #[test]
    fn an_overlay_answers_nothing_rather_than_something_wrong() {
        // The menu covered the line: no candidate, so the row keeps the file it had.
        assert_eq!(read(COMMAND_LINE_MENU), Open::NoStatusLine);
        // The picker did not cover it, and its own `│` borders do not fool the split.
        assert_eq!(read(FILE_PICKER), file("oko-9.txt"));
    }

    #[test]
    fn two_candidates_are_no_answer_rather_than_the_first() {
        // The uniqueness rule, which is the counterfeit defence: `hx`, never `hx fake.rs`.
        assert_eq!(read(GUTTERLESS_COUNTERFEIT), Open::NoStatusLine);
    }

    #[test]
    fn a_status_line_this_cannot_match_produces_nothing() {
        // §2.7's condition as a test: a customised status line costs the file, not the truth.
        assert_eq!(read(CUSTOM_STATUSLINE), Open::NoStatusLine);
    }

    #[test]
    fn the_spinner_cell_is_counted_and_not_trimmed() {
        assert_eq!(read(SPINNER), file("watch.rs"));
    }

    #[test]
    fn an_empty_screen_is_no_answer() {
        assert_eq!(read(EMPTY_SCREEN), Open::NoStatusLine);
        // And no rows at all, which is what `src/iterm/client.rs:screen` returns for a pane
        // that closed between its last update and the read.
        assert_eq!(open_file(&[]), Open::NoStatusLine);
    }
}
