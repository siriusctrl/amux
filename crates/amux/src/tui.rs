use std::{
    io::{self, Write},
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::{model::Session, tmux};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(150);

pub fn run() -> Result<Option<String>> {
    let mut stdout = io::stdout();
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut cleanup = TerminalCleanup::active();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    let result = run_loop(&mut terminal);

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .ok();
    cleanup.disarm();
    terminal.show_cursor().ok();

    result
}

struct TerminalCleanup {
    active: bool,
}

impl TerminalCleanup {
    fn active() -> Self {
        Self { active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        disable_raw_mode().ok();
        let mut stdout = io::stdout();
        execute!(stdout, DisableMouseCapture, LeaveAlternateScreen).ok();
        stdout.flush().ok();
    }
}

#[derive(Debug)]
struct TuiState {
    sessions: Vec<Session>,
    selected: usize,
    message: String,
}

impl TuiState {
    fn refresh(&mut self) {
        match tmux::list_sessions() {
            Ok(sessions) => {
                self.sessions = sessions;
                self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
                self.message = format!("{} local sessions", self.sessions.len());
            }
            Err(error) => {
                self.sessions.clear();
                self.selected = 0;
                self.message = format!("failed to list sessions: {error}");
            }
        }
    }

    fn selected_session_name(&self) -> Option<String> {
        self.sessions
            .get(self.selected)
            .map(|session| session.name.clone())
    }

    fn select_next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.sessions.len() - 1);
    }

    fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn select_row(&mut self, row: usize) {
        if row < self.sessions.len() {
            self.selected = row;
        }
    }
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<Option<String>> {
    let mut state = TuiState {
        sessions: Vec::new(),
        selected: 0,
        message: "loading sessions".to_owned(),
    };
    state.refresh();
    let mut dirty = true;
    let mut session_area = Rect::default();

    loop {
        if dirty {
            session_area = draw(terminal, &state)?;
            dirty = false;
        }

        if !event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal event")? {
            continue;
        }

        match event::read().context("failed to read terminal event")? {
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            Event::Key(key) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                KeyCode::Char('r') => {
                    state.refresh();
                    dirty = true;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    state.select_next();
                    dirty = true;
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.select_previous();
                    dirty = true;
                }
                KeyCode::Enter => return Ok(state.selected_session_name()),
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => {
                    state.select_next();
                    dirty = true;
                }
                MouseEventKind::ScrollUp => {
                    state.select_previous();
                    dirty = true;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(row) = session_row_from_mouse(session_area, mouse.row) {
                        state.select_row(row);
                        dirty = true;
                    }
                }
                _ => {}
            },
            Event::Resize(_, _) => dirty = true,
            _ => {}
        }
    }
}

fn draw(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, state: &TuiState) -> Result<Rect> {
    let mut session_area = Rect::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let [body, footer] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
            let [sessions, details] =
                Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
                    .areas(body);
            session_area = sessions;

            let title = format!(" amux local | {} sessions ", state.sessions.len());
            let items = if state.sessions.is_empty() {
                vec![ListItem::new(Line::from(vec![Span::styled(
                    "No tmux sessions. Use `amux new <name> -- <command>`.",
                    Style::default().fg(Color::DarkGray),
                )]))]
            } else {
                state
                    .sessions
                    .iter()
                    .map(|session| {
                        let status_style = if session.attached {
                            Style::default().fg(Color::Green)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("{:<20}", session.name),
                                Style::default().add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(session.display_status(), status_style),
                            Span::raw(format!("  {}w", session.windows)),
                        ]))
                    })
                    .collect()
            };

            let mut list_state = ListState::default();
            if !state.sessions.is_empty() {
                list_state.select(Some(state.selected));
            }

            frame.render_stateful_widget(
                List::new(items)
                    .block(
                        Block::default()
                            .title(title)
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::DarkGray)),
                    )
                    .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
                    .highlight_symbol("> "),
                sessions,
                &mut list_state,
            );

            frame.render_widget(render_details(state), details);
            frame.render_widget(
                Paragraph::new(format!(
                    " {} | q/Esc quit | r refresh | j/k select | Enter attach | mouse click/wheel ",
                    state.message
                ))
                .style(Style::default().fg(Color::DarkGray)),
                footer,
            );
        })
        .context("failed to draw terminal frame")?;
    Ok(session_area)
}

fn render_details(state: &TuiState) -> Paragraph<'static> {
    let lines = match state.sessions.get(state.selected) {
        Some(session) => vec![
            Line::from(vec![
                Span::styled("session ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    session.name.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(format!("id:       {}", session.id)),
            Line::from(format!("windows:  {}", session.windows)),
            Line::from(format!("status:   {}", session.display_status())),
            Line::from(""),
            Line::from("Enter attaches with tmux for now."),
            Line::from("Future panes and agent status will live here."),
        ],
        None => vec![
            Line::from("No session selected."),
            Line::from(""),
            Line::from("Create one with:"),
            Line::from("amux new work -- codex"),
        ],
    };

    Paragraph::new(lines).block(
        Block::default()
            .title(" details ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
}

fn session_row_from_mouse(area: Rect, row: u16) -> Option<usize> {
    let first_item_row = area.y.saturating_add(1);
    let last_item_row = area.y.saturating_add(area.height).saturating_sub(1);
    if row < first_item_row || row >= last_item_row {
        return None;
    }
    Some(usize::from(row - first_item_row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_rows_map_inside_list_block() {
        let area = Rect {
            x: 0,
            y: 2,
            width: 40,
            height: 10,
        };

        assert_eq!(session_row_from_mouse(area, 2), None);
        assert_eq!(session_row_from_mouse(area, 3), Some(0));
        assert_eq!(session_row_from_mouse(area, 5), Some(2));
        assert_eq!(session_row_from_mouse(area, 11), None);
    }
}
