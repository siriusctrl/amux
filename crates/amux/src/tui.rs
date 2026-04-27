use std::{
    collections::HashSet,
    env,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEventKind,
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

use crate::{
    model::{Pane, Session, SplitDirection},
    tmux,
};

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
    selected_session: usize,
    panes: Vec<Pane>,
    selected_pane: usize,
    selected_launch: usize,
    focus: Focus,
    command_mode: bool,
    message: String,
    current_dir: PathBuf,
}

impl TuiState {
    fn new() -> Self {
        let mut state = Self {
            sessions: Vec::new(),
            selected_session: 0,
            panes: Vec::new(),
            selected_pane: 0,
            selected_launch: 0,
            focus: Focus::Sessions,
            command_mode: false,
            message: "loading sessions".to_owned(),
            current_dir: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        state.refresh_sessions();
        state
    }

    fn refresh_sessions(&mut self) {
        let previous_session = self.selected_session_name();
        let previous_pane = self.selected_pane_id();
        match tmux::list_sessions() {
            Ok(sessions) => {
                self.sessions = sessions;
                self.selected_session = select_index_by_name(
                    &self.sessions,
                    previous_session.as_deref(),
                    self.selected_session,
                );
                self.refresh_panes(previous_pane);
                if self.sessions.is_empty() {
                    self.focus = Focus::Launcher;
                } else if self.focus == Focus::Launcher {
                    self.focus = Focus::Sessions;
                }
                self.message = format!(
                    "{} local sessions, {} panes",
                    self.sessions.len(),
                    self.panes.len()
                );
            }
            Err(error) => {
                self.sessions.clear();
                self.panes.clear();
                self.selected_session = 0;
                self.selected_pane = 0;
                self.focus = Focus::Launcher;
                self.message = format!("failed to list sessions: {error}");
            }
        }
    }

    fn refresh_panes(&mut self, preferred_pane: Option<String>) {
        let Some(session) = self.selected_session_name() else {
            self.panes.clear();
            self.selected_pane = 0;
            return;
        };

        match tmux::list_panes(&session) {
            Ok(panes) => {
                self.panes = panes;
                self.selected_pane =
                    select_pane_index(&self.panes, preferred_pane.as_deref(), self.selected_pane);
            }
            Err(error) => {
                self.panes.clear();
                self.selected_pane = 0;
                self.message = format!("failed to list panes for {session}: {error}");
            }
        }
    }

    fn selected_session_name(&self) -> Option<String> {
        self.sessions
            .get(self.selected_session)
            .map(|session| session.name.clone())
    }

    fn selected_pane_id(&self) -> Option<String> {
        self.panes
            .get(self.selected_pane)
            .map(|pane| pane.id.clone())
    }

    fn select_next_session(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        self.selected_session = (self.selected_session + 1).min(self.sessions.len() - 1);
        self.focus = Focus::Sessions;
        self.refresh_panes(None);
    }

    fn select_previous_session(&mut self) {
        self.selected_session = self.selected_session.saturating_sub(1);
        self.focus = Focus::Sessions;
        self.refresh_panes(None);
    }

    fn select_session_row(&mut self, row: usize) {
        if row < self.sessions.len() {
            self.selected_session = row;
            self.focus = Focus::Sessions;
            self.refresh_panes(None);
            if let Some(name) = self.selected_session_name() {
                self.message = format!("selected session {name}");
            }
        }
    }

    fn select_next_pane(&mut self) {
        if self.panes.is_empty() {
            self.select_next_launch();
            return;
        }
        self.selected_pane = (self.selected_pane + 1).min(self.panes.len() - 1);
        self.focus = Focus::Panes;
        self.select_current_pane_in_tmux();
    }

    fn select_previous_pane(&mut self) {
        if self.panes.is_empty() {
            self.select_previous_launch();
            return;
        }
        self.selected_pane = self.selected_pane.saturating_sub(1);
        self.focus = Focus::Panes;
        self.select_current_pane_in_tmux();
    }

    fn select_pane_row(&mut self, row: usize) {
        if self.panes.is_empty() {
            self.select_launch_row(row);
            return;
        }
        if row < self.panes.len() {
            self.selected_pane = row;
            self.focus = Focus::Panes;
            self.select_current_pane_in_tmux();
        }
    }

    fn select_current_pane_in_tmux(&mut self) {
        let Some(pane_id) = self.selected_pane_id() else {
            return;
        };

        match tmux::select_pane(&pane_id) {
            Ok(()) => {
                for pane in &mut self.panes {
                    pane.active = pane.id == pane_id;
                }
                self.message = format!("selected pane {pane_id}");
            }
            Err(error) => {
                self.message = format!("failed to select pane {pane_id}: {error}");
            }
        }
    }

    fn split_selected_pane(&mut self, direction: SplitDirection) {
        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        match tmux::split_pane(&pane_id, direction) {
            Ok(()) => {
                self.refresh_panes(None);
                self.message = match direction {
                    SplitDirection::Right => "split pane right".to_owned(),
                    SplitDirection::Down => "split pane down".to_owned(),
                };
            }
            Err(error) => {
                self.message = format!("failed to split pane {pane_id}: {error}");
            }
        }
    }

    fn close_selected_pane(&mut self) {
        if self.panes.len() <= 1 {
            self.message = "not closing the last pane in a session".to_owned();
            return;
        }

        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        match tmux::kill_pane(&pane_id) {
            Ok(()) => {
                self.refresh_panes(None);
                self.message = format!("closed pane {pane_id}");
            }
            Err(error) => {
                self.message = format!("failed to close pane {pane_id}: {error}");
            }
        }
    }

    fn toggle_focus(&mut self) {
        if self.sessions.is_empty() {
            self.focus = Focus::Launcher;
            return;
        }
        self.focus = match self.focus {
            Focus::Sessions => Focus::Panes,
            Focus::Panes => Focus::Sessions,
            Focus::Launcher => Focus::Sessions,
        };
    }

    fn select_next_launch(&mut self) {
        self.selected_launch = (self.selected_launch + 1).min(LAUNCH_ACTIONS.len() - 1);
        self.focus = Focus::Launcher;
    }

    fn select_previous_launch(&mut self) {
        self.selected_launch = self.selected_launch.saturating_sub(1);
        self.focus = Focus::Launcher;
    }

    fn select_launch_row(&mut self, row: usize) {
        if row < LAUNCH_ACTIONS.len() {
            self.selected_launch = row;
            self.focus = Focus::Launcher;
            self.message = format!("selected {}", LAUNCH_ACTIONS[row].title());
        }
    }

    fn launch_selected(&mut self) -> Option<String> {
        self.launch_action(LAUNCH_ACTIONS[self.selected_launch])
    }

    fn launch_action(&mut self, _action: LaunchAction) -> Option<String> {
        let base_name = workspace_base_name(&self.current_dir);
        let name = unique_session_name(&base_name, &self.sessions);

        match tmux::create_session(&name, Some(&self.current_dir), &[]) {
            Ok(()) => {
                self.refresh_sessions();
                self.message = format!("created {name}");
                Some(name)
            }
            Err(error) => {
                self.message = format!("failed to create {name}: {error}");
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Sessions,
    Panes,
    Launcher,
}

#[derive(Debug, Clone, Default)]
struct Hitboxes {
    sessions: Rect,
    panes: Rect,
    buttons: Vec<ButtonHitbox>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ButtonHitbox {
    area: Rect,
    action: ButtonAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonAction {
    NewSession,
    Open,
    SplitRight,
    SplitDown,
    ClosePane,
    Refresh,
}

impl ButtonAction {
    fn label(self) -> &'static str {
        match self {
            ButtonAction::NewSession => "New",
            ButtonAction::Open => "Open",
            ButtonAction::SplitRight => "Right",
            ButtonAction::SplitDown => "Down",
            ButtonAction::ClosePane => "Close",
            ButtonAction::Refresh => "Refresh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchAction {
    Session,
}

impl LaunchAction {
    fn title(self) -> &'static str {
        match self {
            LaunchAction::Session => "Start Session",
        }
    }

    fn description(self) -> &'static str {
        match self {
            LaunchAction::Session => "Create a persistent shell here and open it.",
        }
    }
}

const LAUNCH_ACTIONS: [LaunchAction; 1] = [LaunchAction::Session];

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<Option<String>> {
    let mut state = TuiState::new();
    let mut dirty = true;
    let mut hitboxes = Hitboxes::default();

    loop {
        if dirty {
            hitboxes = draw(terminal, &state)?;
            dirty = false;
        }

        if !event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal event")? {
            continue;
        }

        match event::read().context("failed to read terminal event")? {
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            Event::Key(key) => match key.code {
                KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.command_mode = true;
                    state.message = "command mode".to_owned();
                    dirty = true;
                }
                KeyCode::Esc if state.command_mode => {
                    state.command_mode = false;
                    state.message = "command mode canceled".to_owned();
                    dirty = true;
                }
                KeyCode::Esc => return Ok(None),
                KeyCode::Tab => {
                    state.toggle_focus();
                    dirty = true;
                }
                KeyCode::Down => {
                    match state.focus {
                        Focus::Sessions => state.select_next_session(),
                        Focus::Panes => state.select_next_pane(),
                        Focus::Launcher => state.select_next_launch(),
                    }
                    dirty = true;
                }
                KeyCode::Up => {
                    match state.focus {
                        Focus::Sessions => state.select_previous_session(),
                        Focus::Panes => state.select_previous_pane(),
                        Focus::Launcher => state.select_previous_launch(),
                    }
                    dirty = true;
                }
                KeyCode::Char(ch) if state.command_mode => match handle_command_key(&mut state, ch)
                {
                    CommandKeyAction::Continue => dirty = true,
                    CommandKeyAction::Open(session) => return Ok(Some(session)),
                    CommandKeyAction::Quit => return Ok(None),
                },
                KeyCode::Enter if state.sessions.is_empty() => {
                    if let Some(session) = state.launch_selected() {
                        return Ok(Some(session));
                    }
                    dirty = true;
                }
                KeyCode::Enter => return Ok(state.selected_session_name()),
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => {
                    if rect_contains(hitboxes.panes, mouse.column, mouse.row) {
                        state.select_next_pane();
                    } else {
                        state.select_next_session();
                    }
                    dirty = true;
                }
                MouseEventKind::ScrollUp => {
                    if rect_contains(hitboxes.panes, mouse.column, mouse.row) {
                        state.select_previous_pane();
                    } else {
                        state.select_previous_session();
                    }
                    dirty = true;
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(action) = hit_button(&hitboxes.buttons, mouse.column, mouse.row) {
                        if let Some(session) = activate_button(&mut state, action) {
                            return Ok(Some(session));
                        }
                        dirty = true;
                    } else if rect_contains(hitboxes.sessions, mouse.column, mouse.row) {
                        if let Some(row) = row_from_mouse(hitboxes.sessions, mouse.row) {
                            state.select_session_row(row);
                            dirty = true;
                        }
                    } else if rect_contains(hitboxes.panes, mouse.column, mouse.row)
                        && let Some(row) = row_from_mouse(hitboxes.panes, mouse.row)
                    {
                        if state.sessions.is_empty() {
                            state.select_launch_row(row / 2);
                            if let Some(session) = state.launch_selected() {
                                return Ok(Some(session));
                            }
                        } else {
                            state.select_pane_row(row);
                        }
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

fn activate_button(state: &mut TuiState, action: ButtonAction) -> Option<String> {
    if !button_enabled(state, action) {
        state.message = format!("{} is unavailable", action.label());
        return None;
    }

    match action {
        ButtonAction::NewSession => state.launch_action(LaunchAction::Session),
        ButtonAction::Open => state.selected_session_name(),
        ButtonAction::SplitRight => {
            state.split_selected_pane(SplitDirection::Right);
            None
        }
        ButtonAction::SplitDown => {
            state.split_selected_pane(SplitDirection::Down);
            None
        }
        ButtonAction::ClosePane => {
            state.close_selected_pane();
            None
        }
        ButtonAction::Refresh => {
            state.refresh_sessions();
            None
        }
    }
}

enum CommandKeyAction {
    Continue,
    Open(String),
    Quit,
}

fn handle_command_key(state: &mut TuiState, ch: char) -> CommandKeyAction {
    match ch {
        'n' => {
            state.command_mode = false;
            state
                .launch_action(LaunchAction::Session)
                .map(CommandKeyAction::Open)
                .unwrap_or(CommandKeyAction::Continue)
        }
        'a' => {
            state.command_mode = false;
            state
                .selected_session_name()
                .map(CommandKeyAction::Open)
                .unwrap_or_else(|| {
                    state.message = "no session selected".to_owned();
                    CommandKeyAction::Continue
                })
        }
        'v' | '|' => {
            state.command_mode = false;
            state.split_selected_pane(SplitDirection::Right);
            CommandKeyAction::Continue
        }
        'h' | '-' => {
            state.command_mode = false;
            state.split_selected_pane(SplitDirection::Down);
            CommandKeyAction::Continue
        }
        'x' => {
            state.command_mode = false;
            state.close_selected_pane();
            CommandKeyAction::Continue
        }
        'r' => {
            state.command_mode = false;
            state.refresh_sessions();
            CommandKeyAction::Continue
        }
        'q' => CommandKeyAction::Quit,
        _ => {
            state.message = format!("unknown command: {ch}");
            CommandKeyAction::Continue
        }
    }
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &TuiState,
) -> Result<Hitboxes> {
    let mut hitboxes = Hitboxes::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let [body, footer] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
            let [sessions, workspace] =
                Layout::horizontal([Constraint::Percentage(34), Constraint::Percentage(66)])
                    .areas(body);
            let [toolbar, panes, details] = Layout::vertical([
                Constraint::Length(4),
                Constraint::Min(8),
                Constraint::Length(8),
            ])
            .areas(workspace);

            hitboxes.sessions = sessions;
            hitboxes.panes = panes;
            hitboxes.buttons = toolbar_buttons(toolbar);

            frame.render_stateful_widget(
                render_sessions(state),
                sessions,
                &mut session_list_state(state),
            );
            frame.render_widget(render_toolbar(state), toolbar);
            frame.render_stateful_widget(render_panes(state), panes, &mut pane_list_state(state));
            frame.render_widget(render_details(state), details);
            let footer_text = footer_text(state);
            frame.render_widget(
                Paragraph::new(footer_text).style(Style::default().fg(Color::DarkGray)),
                footer,
            );
        })
        .context("failed to draw terminal frame")?;
    Ok(hitboxes)
}

fn footer_text(state: &TuiState) -> String {
    if state.command_mode {
        return " COMMAND | n new | a open | v right | h down | x close | r refresh | q quit | Esc cancel ".to_owned();
    }

    if state.sessions.is_empty() {
        " Enter start | click starter | Ctrl-A commands ".to_owned()
    } else {
        format!(
            " {} | Ctrl-A commands | Tab focus | Enter open | mouse controls ",
            state.message
        )
    }
}

fn render_sessions(state: &TuiState) -> List<'static> {
    let title = format!(" sessions | {} ", focus_label(state.focus, Focus::Sessions));
    let items = if state.sessions.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No sessions yet. Choose a starter on the right.",
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
                        format!("{:<18}", truncate(&session.name, 18)),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(session.display_status(), status_style),
                    Span::raw(format!("  {}w", session.windows)),
                ]))
            })
            .collect()
    };

    List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style(state.focus, Focus::Sessions)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("> ")
}

fn render_toolbar(state: &TuiState) -> Paragraph<'static> {
    let launch_actions = [ButtonAction::NewSession];
    let pane_actions = [
        ButtonAction::Open,
        ButtonAction::SplitRight,
        ButtonAction::SplitDown,
        ButtonAction::ClosePane,
        ButtonAction::Refresh,
    ];

    let lines = vec![
        render_button_line(state, &launch_actions),
        render_button_line(state, &pane_actions),
    ];

    Paragraph::new(lines).block(
        Block::default()
            .title(" actions ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
}

fn render_panes(state: &TuiState) -> List<'static> {
    if state.sessions.is_empty() {
        return render_launcher(state);
    }

    let title = format!(" panes | {} ", focus_label(state.focus, Focus::Panes));
    let items = if state.panes.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No panes for the selected session.",
            Style::default().fg(Color::DarkGray),
        )]))]
    } else {
        state
            .panes
            .iter()
            .map(|pane| {
                let status_style = if pane.active {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("#{} {:<8}", pane.index, pane.display_status()),
                        status_style,
                    ),
                    Span::styled(
                        format!("{:<14}", truncate(&pane.current_command, 14)),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(
                        " {:>3}x{:<3} @{},{} ",
                        pane.width, pane.height, pane.left, pane.top
                    )),
                    Span::styled(truncate(&pane.current_path, 42), Style::default()),
                ]))
            })
            .collect()
    };

    List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style(state.focus, Focus::Panes)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("> ")
}

fn render_details(state: &TuiState) -> Paragraph<'static> {
    let lines = match (
        state.sessions.get(state.selected_session),
        state.panes.get(state.selected_pane),
    ) {
        (Some(session), Some(pane)) => vec![
            Line::from(vec![
                Span::styled("session ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    session.name.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("pane ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    pane.id.clone(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(format!(
                "cmd: {}    cwd: {}",
                pane.current_command, pane.current_path
            )),
            Line::from(format!(
                "layout: {}x{} at {},{}    status: {}",
                pane.width,
                pane.height,
                pane.left,
                pane.top,
                pane.display_status()
            )),
        ],
        (Some(session), None) => vec![
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
            Line::from("No pane data is available for this session."),
        ],
        _ if state.sessions.is_empty() => vec![
            Line::from(vec![
                Span::styled("workspace ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    state.current_dir.display().to_string(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from("Pick a starter to create and open a local session."),
            Line::from("The new session uses your default shell."),
        ],
        _ => vec![
            Line::from("No session selected."),
            Line::from(""),
            Line::from("Choose Start Session to create one."),
        ],
    };

    Paragraph::new(lines).block(
        Block::default()
            .title(" details ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
}

fn session_list_state(state: &TuiState) -> ListState {
    let mut list_state = ListState::default();
    if !state.sessions.is_empty() {
        list_state.select(Some(state.selected_session));
    }
    list_state
}

fn pane_list_state(state: &TuiState) -> ListState {
    let mut list_state = ListState::default();
    if state.sessions.is_empty() {
        list_state.select(Some(state.selected_launch));
    } else if !state.panes.is_empty() {
        list_state.select(Some(state.selected_pane));
    }
    list_state
}

fn render_button_line(state: &TuiState, actions: &[ButtonAction]) -> Line<'static> {
    let mut spans = Vec::new();
    for action in actions {
        spans.push(Span::styled(
            button_label(*action),
            button_style(state, *action),
        ));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn render_launcher(state: &TuiState) -> List<'static> {
    let title = format!(" start | {} ", focus_label(state.focus, Focus::Launcher));
    let items = LAUNCH_ACTIONS
        .iter()
        .map(|action| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    action.title(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    action.description(),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect::<Vec<_>>();

    List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style(state.focus, Focus::Launcher)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol("> ")
}

fn select_index_by_name(sessions: &[Session], name: Option<&str>, fallback: usize) -> usize {
    if sessions.is_empty() {
        return 0;
    }
    name.and_then(|name| sessions.iter().position(|session| session.name == name))
        .unwrap_or_else(|| fallback.min(sessions.len() - 1))
}

fn select_pane_index(panes: &[Pane], preferred: Option<&str>, fallback: usize) -> usize {
    if panes.is_empty() {
        return 0;
    }
    preferred
        .and_then(|pane_id| panes.iter().position(|pane| pane.id == pane_id))
        .or_else(|| panes.iter().position(|pane| pane.active))
        .unwrap_or_else(|| fallback.min(panes.len() - 1))
}

fn toolbar_buttons(area: Rect) -> Vec<ButtonHitbox> {
    let launch_actions = [ButtonAction::NewSession];
    let pane_actions = [
        ButtonAction::Open,
        ButtonAction::SplitRight,
        ButtonAction::SplitDown,
        ButtonAction::ClosePane,
        ButtonAction::Refresh,
    ];
    let rows = [&launch_actions[..], &pane_actions[..]];
    let mut buttons = Vec::new();

    for (row_index, actions) in rows.iter().enumerate() {
        let mut x = area.x.saturating_add(2);
        let y = area.y.saturating_add(1 + row_index as u16);

        for action in *actions {
            let width = button_label(*action).chars().count() as u16;
            buttons.push(ButtonHitbox {
                area: Rect {
                    x,
                    y,
                    width,
                    height: 1,
                },
                action: *action,
            });
            x = x.saturating_add(width).saturating_add(1);
        }
    }

    buttons
}

fn hit_button(buttons: &[ButtonHitbox], column: u16, row: u16) -> Option<ButtonAction> {
    buttons
        .iter()
        .find(|button| rect_contains(button.area, column, row))
        .map(|button| button.action)
}

fn row_from_mouse(area: Rect, row: u16) -> Option<usize> {
    let first_item_row = area.y.saturating_add(1);
    let last_item_row = area.y.saturating_add(area.height).saturating_sub(1);
    if row < first_item_row || row >= last_item_row {
        return None;
    }
    Some(usize::from(row - first_item_row))
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn button_label(action: ButtonAction) -> String {
    format!("[{}]", action.label())
}

fn button_style(state: &TuiState, action: ButtonAction) -> Style {
    if button_enabled(state, action) {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn button_enabled(state: &TuiState, action: ButtonAction) -> bool {
    match action {
        ButtonAction::NewSession => true,
        ButtonAction::Open => !state.sessions.is_empty(),
        ButtonAction::SplitRight | ButtonAction::SplitDown => !state.panes.is_empty(),
        ButtonAction::ClosePane => state.panes.len() > 1,
        ButtonAction::Refresh => true,
    }
}

fn border_style(current: Focus, target: Focus) -> Style {
    if current == target {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn focus_label(current: Focus, target: Focus) -> &'static str {
    if current == target {
        "focused"
    } else {
        "click"
    }
}

fn workspace_base_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("amux");
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();

    if sanitized.is_empty() {
        "amux".to_owned()
    } else {
        sanitized
    }
}

fn unique_session_name(base: &str, sessions: &[Session]) -> String {
    let names = sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect::<HashSet<_>>();
    if !names.contains(base) {
        return base.to_owned();
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }

    unreachable!("unbounded suffix search always returns")
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        truncated
    } else {
        value.to_owned()
    }
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

        assert_eq!(row_from_mouse(area, 2), None);
        assert_eq!(row_from_mouse(area, 3), Some(0));
        assert_eq!(row_from_mouse(area, 5), Some(2));
        assert_eq!(row_from_mouse(area, 11), None);
    }

    #[test]
    fn toolbar_buttons_have_clickable_hitboxes() {
        let buttons = toolbar_buttons(Rect {
            x: 10,
            y: 4,
            width: 80,
            height: 4,
        });

        assert_eq!(hit_button(&buttons, 12, 5), Some(ButtonAction::NewSession));
        assert_eq!(hit_button(&buttons, 12, 6), Some(ButtonAction::Open));
        assert_eq!(hit_button(&buttons, 21, 6), Some(ButtonAction::SplitRight));
        assert_eq!(hit_button(&buttons, 12, 4), None);
    }

    #[test]
    fn workspace_names_are_safe_tmux_session_names() {
        assert_eq!(workspace_base_name(Path::new("/root/towerlab")), "towerlab");
        assert_eq!(workspace_base_name(Path::new("/tmp/my repo")), "my-repo");
    }

    #[test]
    fn generated_session_names_avoid_existing_sessions() {
        let sessions = vec![
            Session {
                id: "$0".to_owned(),
                name: "towerlab".to_owned(),
                windows: 1,
                attached: false,
            },
            Session {
                id: "$1".to_owned(),
                name: "towerlab-2".to_owned(),
                windows: 1,
                attached: false,
            },
        ];

        assert_eq!(unique_session_name("towerlab", &sessions), "towerlab-3");
        assert_eq!(unique_session_name("picoagent", &sessions), "picoagent");
    }

    #[test]
    fn pane_selection_prefers_active_pane() {
        let panes = vec![
            Pane {
                id: "%1".to_owned(),
                index: 0,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 40,
                height: 24,
                left: 0,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
            Pane {
                id: "%2".to_owned(),
                index: 1,
                active: true,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 40,
                height: 24,
                left: 41,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
        ];

        assert_eq!(select_pane_index(&panes, None, 0), 1);
        assert_eq!(select_pane_index(&panes, Some("%1"), 0), 0);
    }
}
