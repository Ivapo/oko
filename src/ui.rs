//! The dashboard: a table of the window's sessions, and the two keys that matter.
//!
//! ```text
//!  Oko — window 0                                        5 rows
//!  ────────────────────────────────────────────────────────────
//!    tab  process           status      where
//!  ▸ 1    claude            ● waiting   ~/dev/main/oko
//!    2    claude            ◐ working   ~/dev/main/spec-driven-dev
//!    3    claude            ◌ stale     ~/dev/main/mdview
//!    4    nvim                          ~/dev/main/oko/src
//!  ────────────────────────────────────────────────────────────
//!  ↵ jump    ↑↓ select    q quit
//! ```
//!
//! **Selection is a session id, not a row number.** Closing a tab above the selection must
//! not re-point Enter at a neighbour: a wrong jump is the failure §2.7 argues is worse than
//! no answer at all.
//!
//! Row 4 is a plain tab: a process name, a directory, and no status, because nothing reports
//! one for it. Rows 1–3 are Claude tabs, and they read `claude` because a status file exists
//! for them (OQ-2) — never because of the job name, which for those rows is some descendant
//! of `claude` and is not an identity test.

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
use oko::status::Status;

/// Everything the event loop consumes, from either thread.
pub enum AppEvent {
    Terminal(TermEvent),
    Iterm(ItermEvent),
}

pub struct App {
    snapshot: Snapshot,
    /// The selected *session*, so a changing row set cannot move the selection onto a
    /// different one.
    selected: Option<String>,
    status: Option<String>,
    table: TableState,
}

impl App {
    pub fn new(snapshot: Snapshot) -> Self {
        let selected = snapshot.rows.first().map(|r| r.session_id.clone());
        App { snapshot, selected, status: None, table: TableState::new() }
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
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
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

    let header = TableRow::new(["tab", "process", "status", "where"].map(|h| Cell::from(h).dim()));
    let rows: Vec<TableRow> = app.snapshot.rows.iter().map(render_row).collect();
    let widths = [
        Constraint::Length(4),
        Constraint::Length(17),
        Constraint::Length(10),
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

    let keys = match &app.status {
        Some(message) => Line::from(format!(" {message}")).style(Style::new().fg(Color::Red)),
        None => Line::from(" ↵ jump    ↑↓ select    q quit").dim(),
    };
    frame.render_widget(Paragraph::new(keys), footer);
}

fn render_row(row: &Row) -> TableRow<'static> {
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
        Cell::from(process),
        render_status(row.status),
        Cell::from(row.path.as_deref().map_or_else(|| "-".to_string(), abbreviate_home)),
    ])
}

/// The glyph and the word, as §1 draws them. A plain tab gets an empty cell rather than a
/// dash: it has no status because nothing reports one for it, which is not the same as a
/// value that failed to arrive.
fn render_status(status: Option<Status>) -> Cell<'static> {
    let Some(status) = status else {
        return Cell::from("");
    };
    let colour = match status {
        Status::Working => Color::Cyan,
        Status::Waiting => Color::Yellow,
        Status::Ready => Color::Green,
        Status::Stale => Color::DarkGray,
    };
    Cell::from(Line::from(vec![
        Span::styled(status.glyph(), Style::new().fg(colour)),
        Span::raw(" "),
        // `stale` is the one status that is not a claim about the agent but about Oko's own
        // knowledge, so it is dimmed rather than stated in the same voice as the other three.
        match status {
            Status::Stale => Span::styled(status.word(), Style::new().fg(Color::DarkGray)),
            _ => Span::raw(status.word()),
        },
    ]))
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
