//! The dashboard: a table of the window's sessions, and the two keys that matter.
//!
//! ```text
//!  Oko — window 0                                            5 rows
//!  ────────────────────────────────────────────────────────────────
//!    tab  name             process   status          where
//!  ▸ 1    api work         claude    ● waiting >10m  ~/dev/main/oko
//!    2    spec-driven-dev  claude    ◐ working       ~/dev/main/spec-driven-dev
//!    3    mdview           claude    ◌ stale >30m    ~/dev/main/mdview
//!    4    src              nvim                      ~/dev/main/oko/src
//!  ────────────────────────────────────────────────────────────────
//!  ↵ jump    ↑↓ select    r rename    q quit
//! ```
//!
//! **Selection is a session id, not a row number.** Closing a tab above the selection must
//! not re-point Enter at a neighbour: a wrong jump is the failure §2.7 argues is worse than
//! no answer at all.
//!
//! Row 4 is a plain tab: a name, a process name, a directory, and no status, because nothing
//! reports one for it. Rows 1–3 are Claude tabs, and they read `claude` because a status file
//! exists for them (OQ-2) — never because of the job name, which for those rows is some
//! descendant of `claude` and is not an identity test.
//!
//! Row 1 is named and rows 2–4 are not: an un-named row shows the last component of its
//! directory, which follows a `cd` because it is a description of where the row is rather
//! than a name (§2.10). The age after each status is bucketed, never counted — a live
//! seconds counter would redraw this table every second, forever (§2.11).

use std::sync::mpsc::{Receiver, Sender};

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row as TableRow, Table, TableState};

use oko::iterm::{Cmd, Event as ItermEvent, Row, Snapshot};
use oko::status::{Shown, Status};

/// The status column's width, and it is **counted rather than trusted**.
///
/// `◐ working >10m` is 1 + 1 + 7 + 1 + 4 = 14 cells: the glyphs are East-Asian-Ambiguous and
/// score 1, which the `Length(10)` that held `● waiting` (9) before the age arrived already
/// demonstrated. At 13 ratatui truncates silently, `>10m` renders `>10`, and the gate check
/// for the ladder fails against a correct bucket function. A test below asserts it.
const STATUS_WIDTH: u16 = 14;

/// Everything the event loop consumes, from either thread.
pub enum AppEvent {
    Terminal(TermEvent),
    Iterm(ItermEvent),
}

/// The inline rename, and the only modal state Oko has.
///
/// While it is open `q` types a `q`, `Enter` commits rather than jumping, and `↑↓` do not
/// move the selection.
struct Edit {
    /// Fixed when the editor opened, so a row set changing underneath cannot re-point a
    /// rename at a different session — the same reason the selection is a session id.
    session_id: String,
    buffer: String,
    /// The prefill is *selected*: the first character typed replaces it, as a text field
    /// whose content is highlighted does everywhere else. Without it, renaming a row shown
    /// under its derived default would mean clearing that default by hand first.
    selected: bool,
}

pub struct App {
    snapshot: Snapshot,
    /// The selected *session*, so a changing row set cannot move the selection onto a
    /// different one.
    selected: Option<String>,
    status: Option<String>,
    editing: Option<Edit>,
    table: TableState,
}

impl App {
    pub fn new(snapshot: Snapshot) -> Self {
        let selected = snapshot.rows.first().map(|r| r.session_id.clone());
        App { snapshot, selected, status: None, editing: None, table: TableState::new() }
    }

    fn selected_row(&self) -> Option<&Row> {
        self.snapshot.rows.get(self.selected_index()?)
    }

    /// Opens the rename editor on the selected row, prefilled with what that row shows.
    fn begin_edit(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let buffer = row.name.clone().unwrap_or_default();
        self.editing = Some(Edit {
            session_id: row.session_id.clone(),
            selected: !buffer.is_empty(),
            buffer,
        });
    }

    /// Index of the selected session, and the only place the two representations meet.
    fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.snapshot.rows.iter().position(|r| r.session_id == id)
    }

    fn select_index(&mut self, index: usize) {
        self.selected = self.snapshot.rows.get(index).map(|r| r.session_id.clone());
    }

    fn move_selection(&mut self, delta: isize) {
        if self.snapshot.rows.is_empty() {
            return;
        }
        let last = self.snapshot.rows.len() - 1;
        let next = match self.selected_index() {
            Some(i) => i.saturating_add_signed(delta).min(last),
            None => 0,
        };
        self.select_index(next);
    }

    /// Takes a new row set, keeping the highlight on the same session. When that session is
    /// gone — it was the row that closed — the highlight falls to whatever now occupies its
    /// position, which is the nearest thing to "where you were looking".
    fn apply(&mut self, snapshot: Snapshot) {
        let previous_index = self.selected_index();
        let previous_id = self.selected.clone();
        self.snapshot = snapshot;

        let still_there = previous_id
            .as_deref()
            .is_some_and(|id| self.snapshot.rows.iter().any(|r| r.session_id == id));
        if !still_there {
            let fallback =
                previous_index.unwrap_or(0).min(self.snapshot.rows.len().saturating_sub(1));
            self.select_index(fallback);
        }
    }
}

/// What Enter, ↑↓ and q do; `None` means quit.
enum Action {
    Redraw,
    Jump(String),
    /// A committed rename, or a clear when the name is `None`.
    Rename(String, Option<String>),
    Quit,
}

pub fn run(
    terminal: &mut DefaultTerminal,
    events: &Receiver<AppEvent>,
    commands: &Sender<Cmd>,
    initial: Snapshot,
) -> Result<()> {
    let mut app = App::new(initial);
    terminal.draw(|frame| draw(frame, &mut app))?;

    while let Ok(event) = events.recv() {
        let action = match event {
            AppEvent::Terminal(TermEvent::Key(key)) => on_key(&mut app, key),
            AppEvent::Terminal(_) => Action::Redraw,
            AppEvent::Iterm(ItermEvent::Snapshot(snapshot)) => {
                app.apply(snapshot);
                app.status = None;
                Action::Redraw
            }
            AppEvent::Iterm(ItermEvent::Error(message)) => {
                app.status = Some(message);
                Action::Redraw
            }
        };

        match action {
            Action::Quit => break,
            Action::Jump(session_id) => {
                if commands.send(Cmd::Activate(session_id)).is_err() {
                    app.status = Some("the iTerm2 connection is gone".to_string());
                }
            }
            // Not written into `app.snapshot`: that is a copy, and a name put there would
            // show for one frame and vanish at the next emission. The watcher owns the
            // client, so it owns the write and the row it updates.
            Action::Rename(session_id, name) => {
                if commands.send(Cmd::Rename(session_id, name)).is_err() {
                    app.status = Some("the iTerm2 connection is gone".to_string());
                }
            }
            Action::Redraw => {}
        }
        terminal.draw(|frame| draw(frame, &mut app))?;
    }
    Ok(())
}

fn on_key(app: &mut App, key: KeyEvent) -> Action {
    // Only presses: a key repeat or release would move the selection twice.
    if key.kind != KeyEventKind::Press {
        return Action::Redraw;
    }
    // The mode is checked first and completely: every binding below it is unreachable while
    // the editor is open, which is what "modal" means here.
    if app.editing.is_some() {
        return on_key_editing(app, key);
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('r') => {
            app.begin_edit();
            Action::Redraw
        }
        KeyCode::Char('c' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
        KeyCode::Down | KeyCode::Char('j') => {
            app.move_selection(1);
            Action::Redraw
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.move_selection(-1);
            Action::Redraw
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.select_index(0);
            Action::Redraw
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.select_index(app.snapshot.rows.len().saturating_sub(1));
            Action::Redraw
        }
        KeyCode::Enter => match app.selected.clone() {
            Some(session_id) => Action::Jump(session_id),
            None => Action::Redraw,
        },
        _ => Action::Redraw,
    }
}

/// The rename editor's keys, and nothing else reaches it.
///
/// `^C` and `^D` still quit. That is the one deliberate hole in the mode: a modal state a
/// terminal program cannot be killed out of is worse than the surprise, and `Esc` is the
/// documented way out.
fn on_key_editing(app: &mut App, key: KeyEvent) -> Action {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    if control && matches!(key.code, KeyCode::Char('c' | 'd')) {
        return Action::Quit;
    }

    let Some(edit) = app.editing.as_mut() else {
        return Action::Redraw;
    };
    match key.code {
        KeyCode::Esc => {
            app.editing = None;
            Action::Redraw
        }
        KeyCode::Enter => {
            let edit = app.editing.take().expect("checked just above");
            let name = edit.buffer.trim().to_string();
            // An empty name sets the variable to JSON `null`, which unsets it and returns
            // the row to its derived default. That is the only way back, so it is not
            // optional — and it is why the empty string is not the encoding (OQ-5).
            Action::Rename(edit.session_id, (!name.is_empty()).then_some(name))
        }
        KeyCode::Backspace => {
            if edit.selected {
                edit.buffer.clear();
                edit.selected = false;
            } else {
                edit.buffer.pop();
            }
            Action::Redraw
        }
        KeyCode::Char('u') if control => {
            edit.buffer.clear();
            edit.selected = false;
            Action::Redraw
        }
        // Everything typable is typed, which is the whole of the mode: `q` does not quit,
        // `j` and `k` do not move, `g` and `G` do not jump to the ends.
        KeyCode::Char(c) if !control => {
            if edit.selected {
                edit.buffer.clear();
                edit.selected = false;
            }
            edit.buffer.push(c);
            Action::Redraw
        }
        _ => Action::Redraw,
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let [title, rule_top, body, rule_bottom, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let window = match app.snapshot.window_number {
        Some(n) => format!(" Oko — window {n}"),
        None => " Oko".to_string(),
    };
    let count = match app.snapshot.rows.len() {
        1 => "1 row".to_string(),
        n => format!("{n} rows"),
    };
    let [name, tally] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(count.len() as u16 + 1)])
            .areas(title);
    frame.render_widget(Paragraph::new(window.bold()).style(Style::new().fg(Color::Cyan)), name);
    frame.render_widget(Paragraph::new(Line::from(count).right_aligned()).dim(), tally);

    rule(frame, rule_top);
    rule(frame, rule_bottom);

    let header =
        TableRow::new(["tab", "name", "process", "status", "where"].map(|h| Cell::from(h).dim()));
    // The editor belongs to one session, not to one index: a row set changing while it is
    // open must not move the field onto a neighbour.
    let editing = app.editing.as_ref();
    let rows: Vec<TableRow> = app
        .snapshot
        .rows
        .iter()
        .map(|row| render_row(row, editing.filter(|e| e.session_id == row.session_id)))
        .collect();
    let widths = [
        Constraint::Length(4),
        // The one width nothing else fixes. It holds `spec-driven-dev`.
        Constraint::Length(16),
        Constraint::Length(17),
        Constraint::Length(STATUS_WIDTH),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");
    app.table.select(app.selected_index());
    frame.render_stateful_widget(
        table,
        body.inner(ratatui::layout::Margin::new(1, 0)),
        &mut app.table,
    );

    let keys = match (&app.status, &app.editing) {
        (Some(message), _) => Line::from(format!(" {message}")).style(Style::new().fg(Color::Red)),
        (None, Some(_)) => Line::from(" ↵ commit    esc cancel    ^U clear").dim(),
        (None, None) => Line::from(" ↵ jump    ↑↓ select    r rename    q quit").dim(),
    };
    frame.render_widget(Paragraph::new(keys), footer);
}

fn render_row(row: &Row, edit: Option<&Edit>) -> TableRow<'static> {
    // **A row carrying a status reads `claude`** (OQ-2). The status file has no name field,
    // and §2.2's job name is the *deepest* foreground process — `node` here,
    // `rust-analyzer-pr` there, whatever happened to be deepest when iTerm2 sampled — so it
    // is a display value and never an identity test.
    let process = match row.status {
        Some(_) => "claude".to_string(),
        None => row.process.clone().unwrap_or_else(|| "-".to_string()),
    };
    TableRow::new(vec![
        Cell::from(row.tab.to_string()),
        render_name(row, edit),
        Cell::from(process),
        Cell::from(render_status(row.status)),
        Cell::from(row.path.as_deref().map_or_else(|| "-".to_string(), abbreviate_home)),
    ])
}

/// The name, or the editor when this is the row being renamed.
///
/// `-` for a row with neither a stored name nor a path, the same rule `process` and `where`
/// already use for a value that did not arrive.
fn render_name(row: &Row, edit: Option<&Edit>) -> Cell<'static> {
    let Some(edit) = edit else {
        return Cell::from(row.name.clone().unwrap_or_else(|| "-".to_string()));
    };
    let style = if edit.selected {
        // The prefill, highlighted, because the next character typed replaces it.
        Style::new().add_modifier(Modifier::UNDERLINED)
    } else {
        Style::new()
    };
    Cell::from(Line::from(vec![
        Span::styled(edit.buffer.clone(), style),
        Span::styled("▏", Style::new().fg(Color::Cyan)),
    ]))
}

/// The glyph, the word and the age, as §1 draws them. A plain tab gets an empty cell rather
/// than a dash: it has no status because nothing reports one for it, which is not the same
/// as a value that failed to arrive.
fn render_status(shown: Option<Shown>) -> Line<'static> {
    let Some(Shown { status, age }) = shown else {
        return Line::default();
    };
    let colour = match status {
        Status::Working => Color::Cyan,
        Status::Waiting => Color::Yellow,
        Status::Ready => Color::Green,
        Status::Stale => Color::DarkGray,
    };
    let mut spans = vec![
        Span::styled(status.glyph(), Style::new().fg(colour)),
        Span::raw(" "),
        // `stale` is the one status that is not a claim about the agent but about Oko's own
        // knowledge, so it is dimmed rather than stated in the same voice as the other three.
        match status {
            Status::Stale => Span::styled(status.word(), Style::new().fg(Color::DarkGray)),
            _ => Span::raw(status.word()),
        },
    ];
    // Only once it starts being a question (§2.11), which is why a row that has just changed
    // shows the word alone.
    if let Some(age) = age {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(age.label(), Style::new().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

/// `/Users/me/dev` → `~/dev`, as §1's sketch shows it.
fn abbreviate_home(path: &str) -> String {
    let Ok(home) = std::env::var("HOME") else {
        return path.to_string();
    };
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(&home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

fn rule(frame: &mut ratatui::Frame, area: Rect) {
    frame.render_widget(Paragraph::new("─".repeat(area.width as usize)).dim(), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oko::status::Age;

    #[test]
    fn the_status_column_holds_the_widest_cell_the_ladder_can_produce() {
        // Counted, not trusted. At `STATUS_WIDTH - 1` ratatui truncates silently and `>10m`
        // renders as `>10`, which fails the gate check for the ladder against a correct
        // bucket function — so this is the literal that has to be measured rather than
        // reasoned about.
        let mut widest = 0;
        for status in [Status::Working, Status::Waiting, Status::Ready, Status::Stale] {
            for age in [None, Some(Age::M5), Some(Age::M10), Some(Age::M30), Some(Age::H1)] {
                widest = widest.max(render_status(Some(Shown { status, age })).width());
            }
        }
        // `◐ working >10m` — 1 + 1 + 7 + 1 + 4. The glyphs are East-Asian-Ambiguous and
        // score 1 apiece.
        assert_eq!(widest, 14);
        assert_eq!(widest, usize::from(STATUS_WIDTH));
    }

    #[test]
    fn a_row_with_no_status_renders_an_empty_cell_not_a_dash() {
        // A plain tab has no status because nothing reports one for it, which is not the
        // same as a value that failed to arrive.
        assert_eq!(render_status(None).width(), 0);
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn app_with(name: Option<&str>) -> App {
        App::new(Snapshot {
            window_number: Some(0),
            rows: vec![Row {
                session_id: "S".to_string(),
                tab: 1,
                process: None,
                path: Some("/Users/me/dev/main/oko".to_string()),
                stored_name: None,
                name: name.map(str::to_owned),
                status: None,
            }],
        })
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            on_key(app, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn the_editor_is_modal() {
        let mut app = app_with(Some("oko"));
        on_key(&mut app, key(KeyCode::Char('r')));

        // `q` types a `q` rather than quitting, and `j`/`k`/`g`/`G` type themselves.
        type_str(&mut app, "qjkgG");
        assert!(matches!(on_key(&mut app, key(KeyCode::Down)), Action::Redraw));
        assert_eq!(app.editing.as_ref().unwrap().buffer, "qjkgG");
        // And the selection did not move under it.
        assert_eq!(app.selected.as_deref(), Some("S"));
    }

    #[test]
    fn the_prefill_is_replaced_by_the_first_character_typed() {
        let mut app = app_with(Some("oko"));
        on_key(&mut app, key(KeyCode::Char('r')));
        assert_eq!(app.editing.as_ref().unwrap().buffer, "oko");

        type_str(&mut app, "api work");
        let Action::Rename(id, name) = on_key(&mut app, key(KeyCode::Enter)) else {
            panic!("Enter should commit, not jump");
        };
        assert_eq!(id, "S");
        assert_eq!(name.as_deref(), Some("api work"));
        assert!(app.editing.is_none());
    }

    #[test]
    fn committing_an_empty_name_clears_it() {
        let mut app = app_with(Some("api work"));
        on_key(&mut app, key(KeyCode::Char('r')));
        on_key(&mut app, KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        let Action::Rename(_, name) = on_key(&mut app, key(KeyCode::Enter)) else {
            panic!("Enter should commit");
        };
        // `None` is what becomes JSON `null`, and unsetting is the only way back to the
        // derived default.
        assert_eq!(name, None);
    }

    #[test]
    fn escape_cancels_without_renaming() {
        let mut app = app_with(Some("oko"));
        on_key(&mut app, key(KeyCode::Char('r')));
        type_str(&mut app, "throwaway");
        assert!(matches!(on_key(&mut app, key(KeyCode::Esc)), Action::Redraw));
        assert!(app.editing.is_none());
        // And `q` quits again, the mode being over.
        assert!(matches!(on_key(&mut app, key(KeyCode::Char('q'))), Action::Quit));
    }

    #[test]
    fn an_un_named_row_opens_an_empty_field() {
        // Nothing to select, so nothing is replaced: backspace behaves as backspace.
        let mut app = app_with(None);
        on_key(&mut app, key(KeyCode::Char('r')));
        let edit = app.editing.as_ref().unwrap();
        assert_eq!(edit.buffer, "");
        assert!(!edit.selected);
    }

    /// The table as it actually reaches a terminal, which is the only thing that can catch a
    /// column one cell too narrow: [`STATUS_WIDTH`] being right about the *text* would not
    /// save a layout that gave the cell less room than the text needs.
    fn render(rows: Vec<Row>, width: u16) -> Vec<String> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 10)).unwrap();
        let mut app = App::new(Snapshot { window_number: Some(0), rows });
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()
            })
            .collect()
    }

    fn claude_row(id: &str, tab: usize, name: &str, shown: Shown) -> Row {
        Row {
            session_id: id.to_string(),
            tab,
            process: Some("node".to_string()),
            path: Some("/Users/me/dev/main/spec-driven-dev".to_string()),
            stored_name: Some(name.to_string()),
            name: Some(name.to_string()),
            status: Some(shown),
        }
    }

    #[test]
    fn the_age_is_not_truncated_in_the_drawn_table() {
        let lines = render(
            vec![
                claude_row("A", 1, "api work", Shown {
                    status: Status::Working,
                    age: Some(Age::M10),
                }),
                claude_row("B", 2, "spec-driven-dev", Shown {
                    status: Status::Waiting,
                    age: Some(Age::M30),
                }),
            ],
            100,
        );
        let table = lines.join("\n");

        // The whole point: at one cell less, these read `>10` and `>30`.
        assert!(table.contains("◐ working >10m"), "{table}");
        assert!(table.contains("● waiting >30m"), "{table}");
        // The name column holds the longest realistic derived default.
        assert!(table.contains("spec-driven-dev"), "{table}");
        // Header order, and the column the phase adds.
        assert!(lines[2].contains("tab"), "{table}");
        let header = &lines[2];
        let index = |needle: &str| header.find(needle).unwrap_or_else(|| panic!("{header}"));
        assert!(index("tab") < index("name"));
        assert!(index("name") < index("process"));
        assert!(index("process") < index("status"));
        assert!(index("status") < index("where"));
        // A row carrying a status reads `claude`, never the job name (OQ-2).
        assert!(!table.contains("node"), "{table}");
    }

    #[test]
    fn a_row_under_five_minutes_shows_the_word_alone() {
        let lines = render(
            vec![claude_row("A", 1, "api work", Shown { status: Status::Ready, age: None })],
            100,
        );
        let table = lines.join("\n");
        assert!(table.contains("○ ready"), "{table}");
        assert!(!table.contains('>'), "{table}");
    }

    #[test]
    fn the_editor_draws_in_the_name_cell() {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 10)).unwrap();
        let mut app = App::new(Snapshot {
            window_number: Some(0),
            rows: vec![claude_row("A", 1, "api work", Shown {
                status: Status::Working,
                age: None,
            })],
        });
        on_key(&mut app, key(KeyCode::Char('r')));
        type_str(&mut app, "renamed");
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let table: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n"
            })
            .collect();
        // In the row, not in the footer — and the old name is gone, the prefill having been
        // replaced by the first character typed.
        assert!(table.contains("renamed▏"), "{table}");
        assert!(!table.contains("api work"), "{table}");
        assert!(table.contains("↵ commit"), "{table}");
        assert!(!table.contains("↵ jump"), "{table}");
    }
}
