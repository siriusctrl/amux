use std::{
    collections::HashMap,
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
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
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    model::{Pane, SplitDirection},
    tmux,
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(30);
const REFRESH_INTERVAL: Duration = Duration::from_millis(75);

pub fn run(session: &str) -> Result<()> {
    tmux::list_panes(session).with_context(|| format!("failed to open session {session}"))?;

    let mut stdout = io::stdout();
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut cleanup = SessionTerminalCleanup::active();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )
    .context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
    let result = run_loop(&mut terminal, session);

    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .ok();
    cleanup.disarm();
    terminal.show_cursor().ok();

    result
}

struct SessionTerminalCleanup {
    active: bool,
}

impl SessionTerminalCleanup {
    fn active() -> Self {
        Self { active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for SessionTerminalCleanup {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        disable_raw_mode().ok();
        let mut stdout = io::stdout();
        execute!(
            stdout,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        )
        .ok();
        stdout.flush().ok();
    }
}

#[derive(Debug)]
struct SessionViewState {
    session: String,
    panes: Vec<Pane>,
    selected_pane: usize,
    captures: HashMap<String, Vec<Line<'static>>>,
    scroll_offsets: HashMap<String, usize>,
    command_mode: bool,
    message: String,
}

impl SessionViewState {
    fn new(session: &str) -> Self {
        Self {
            session: session.to_owned(),
            panes: Vec::new(),
            selected_pane: 0,
            captures: HashMap::new(),
            scroll_offsets: HashMap::new(),
            command_mode: false,
            message: "opening session".to_owned(),
        }
    }

    fn refresh(&mut self, body: Rect) -> RefreshStatus {
        let requested_size = tmux_content_size_for_body(body, &self.panes);
        if let Err(error) = tmux::resize_window(&self.session, requested_size.0, requested_size.1) {
            self.message = format!("resize failed: {error}");
        }

        let previous_pane = self.selected_pane_id();
        match tmux::list_panes(&self.session) {
            Ok(mut panes) => {
                if panes.is_empty() {
                    return RefreshStatus::Ended;
                }

                let stable_size = tmux_content_size_for_body(body, &panes);
                if stable_size != requested_size {
                    if let Err(error) =
                        tmux::resize_window(&self.session, stable_size.0, stable_size.1)
                    {
                        self.message = format!("resize failed: {error}");
                    } else if let Ok(stabilized_panes) = tmux::list_panes(&self.session)
                        && !stabilized_panes.is_empty()
                    {
                        panes = stabilized_panes;
                    }
                }

                self.panes = panes;
                self.selected_pane =
                    select_pane_index(&self.panes, previous_pane.as_deref(), self.selected_pane);
                self.prune_scroll_offsets();
                self.refresh_captures(body);
                if self.message == "opening session" {
                    self.message = format!("{} panes", self.panes.len());
                }
                RefreshStatus::Active
            }
            Err(_) => {
                self.panes.clear();
                self.captures.clear();
                self.selected_pane = 0;
                RefreshStatus::Ended
            }
        }
    }

    fn refresh_captures(&mut self, body: Rect) {
        self.captures.clear();
        for pane in &self.panes {
            let area = pane_area(body, pane, &self.panes);
            let height = area.height as usize;
            let scroll_offset = self.scroll_offset(&pane.id);
            match tmux::capture_pane(&pane.id, height, scroll_offset) {
                Ok(capture) => {
                    self.captures
                        .insert(pane.id.clone(), capture_to_lines(&capture));
                }
                Err(error) => {
                    self.captures.insert(
                        pane.id.clone(),
                        vec![Line::from(Span::styled(
                            format!("capture failed: {error}"),
                            Style::default().fg(Color::Red),
                        ))],
                    );
                }
            }
        }
    }

    fn selected_pane_id(&self) -> Option<String> {
        self.panes
            .get(self.selected_pane)
            .map(|pane| pane.id.clone())
    }

    fn select_pane_id(&mut self, pane_id: &str) {
        let Some(index) = self.panes.iter().position(|pane| pane.id == pane_id) else {
            return;
        };
        self.selected_pane = index;
        match tmux::select_pane(pane_id) {
            Ok(()) => self.message = format!("selected pane {pane_id}"),
            Err(error) => self.message = format!("select failed: {error}"),
        }
    }

    fn scroll_offset(&self, pane_id: &str) -> usize {
        self.scroll_offsets.get(pane_id).copied().unwrap_or(0)
    }

    fn scroll_selected(&mut self, delta: isize) {
        let Some(pane_id) = self.selected_pane_id() else {
            return;
        };
        let current = self.scroll_offset(&pane_id);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        };
        if next == 0 {
            self.scroll_offsets.remove(&pane_id);
            self.message = "live view".to_owned();
        } else {
            self.scroll_offsets.insert(pane_id, next);
            self.message = format!("scrollback -{next}");
        }
    }

    fn reset_selected_scroll(&mut self) {
        if let Some(pane_id) = self.selected_pane_id() {
            self.scroll_offsets.remove(&pane_id);
        }
    }

    fn prune_scroll_offsets(&mut self) {
        let pane_ids = self
            .panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<Vec<_>>();
        self.scroll_offsets
            .retain(|pane_id, _| pane_ids.iter().any(|active| active == pane_id));
    }

    fn split_selected_pane(&mut self, direction: SplitDirection) {
        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        match tmux::split_pane(&pane_id, direction) {
            Ok(()) => {
                self.scroll_offsets.clear();
                self.selected_pane = self.panes.len();
                self.message = match direction {
                    SplitDirection::Right => "split pane right".to_owned(),
                    SplitDirection::Down => "split pane down".to_owned(),
                };
            }
            Err(error) => self.message = format!("split failed: {error}"),
        }
    }

    fn close_selected_pane(&mut self) {
        if self.panes.len() <= 1 {
            self.message = "not closing the last pane".to_owned();
            return;
        }

        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        match tmux::kill_pane(&pane_id) {
            Ok(()) => {
                self.scroll_offsets.remove(&pane_id);
                self.selected_pane = self.selected_pane.saturating_sub(1);
                self.message = format!("closed pane {pane_id}");
            }
            Err(error) => self.message = format!("close failed: {error}"),
        }
    }

    fn send_literal(&mut self, text: &str) {
        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        self.reset_selected_scroll();
        if let Err(error) = tmux::send_literal(&pane_id, text) {
            self.message = format!("send failed: {error}");
        }
    }

    fn send_key(&mut self, key: &str) {
        let Some(pane_id) = self.selected_pane_id() else {
            self.message = "no pane selected".to_owned();
            return;
        };

        self.reset_selected_scroll();
        if let Err(error) = tmux::send_key(&pane_id, key) {
            self.message = format!("send failed: {error}");
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SessionHitboxes {
    panes: Vec<PaneHitbox>,
}

#[derive(Debug, Clone)]
struct PaneHitbox {
    pane_id: String,
    area: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshStatus {
    Active,
    Ended,
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, session: &str) -> Result<()> {
    let mut state = SessionViewState::new(session);
    let mut hitboxes = SessionHitboxes::default();
    let mut last_refresh = Instant::now() - REFRESH_INTERVAL;
    let mut dirty = true;

    loop {
        let body = body_area(
            terminal
                .size()
                .context("failed to read terminal size")?
                .into(),
        );
        if dirty || last_refresh.elapsed() >= REFRESH_INTERVAL {
            if state.refresh(body) == RefreshStatus::Ended {
                return Ok(());
            }
            last_refresh = Instant::now();
            dirty = true;
        }

        if dirty {
            hitboxes = draw(terminal, &state)?;
            dirty = false;
        }

        if !event::poll(EVENT_POLL_INTERVAL).context("failed to poll terminal event")? {
            continue;
        }

        match event::read().context("failed to read terminal event")? {
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            Event::Key(key) => {
                if !handle_key(&mut state, key) {
                    return Ok(());
                }
                dirty = true;
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(pane_id) =
                        hit_pane(&hitboxes.panes, mouse.column, mouse.row).map(str::to_owned)
                    {
                        state.select_pane_id(&pane_id);
                        dirty = true;
                    }
                }
                MouseEventKind::ScrollUp => {
                    if let Some(pane_id) =
                        hit_pane(&hitboxes.panes, mouse.column, mouse.row).map(str::to_owned)
                    {
                        state.select_pane_id(&pane_id);
                    }
                    state.scroll_selected(3);
                    dirty = true;
                }
                MouseEventKind::ScrollDown => {
                    if let Some(pane_id) =
                        hit_pane(&hitboxes.panes, mouse.column, mouse.row).map(str::to_owned)
                    {
                        state.select_pane_id(&pane_id);
                    }
                    state.scroll_selected(-3);
                    dirty = true;
                }
                _ => {}
            },
            Event::Paste(text) => {
                state.send_literal(&text);
                dirty = true;
            }
            Event::Resize(_, _) => dirty = true,
            _ => {}
        }
    }
}

fn handle_key(state: &mut SessionViewState, key: KeyEvent) -> bool {
    if state.command_mode {
        return handle_command_key(state, key);
    }

    if matches!(key.code, KeyCode::Char('a' | 'A')) && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        state.command_mode = true;
        state.message = "command mode".to_owned();
        return true;
    }

    forward_key(state, key);
    true
}

fn handle_command_key(state: &mut SessionViewState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc => {
            state.command_mode = false;
            state.message = "command mode canceled".to_owned();
            true
        }
        KeyCode::Char('v' | '|') => {
            state.command_mode = false;
            state.split_selected_pane(SplitDirection::Right);
            true
        }
        KeyCode::Char('h' | '-') => {
            state.command_mode = false;
            state.split_selected_pane(SplitDirection::Down);
            true
        }
        KeyCode::Char('x') => {
            state.command_mode = false;
            state.close_selected_pane();
            true
        }
        KeyCode::Char('r') => {
            state.command_mode = false;
            state.message = "refreshed".to_owned();
            true
        }
        KeyCode::Char('q') => false,
        KeyCode::Char(ch) => {
            state.message = format!("unknown command: {ch}");
            true
        }
        _ => true,
    }
}

fn forward_key(state: &mut SessionViewState, key: KeyEvent) {
    match key.code {
        KeyCode::Char(ch)
            if key.modifiers.contains(KeyModifiers::CONTROL) && ch.is_ascii_alphabetic() =>
        {
            state.send_key(&format!("C-{}", ch.to_ascii_lowercase()));
        }
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::ALT) => {
            state.send_key("Escape");
            state.send_literal(&ch.to_string());
        }
        KeyCode::Char(ch) => state.send_literal(&ch.to_string()),
        KeyCode::Enter => state.send_key("Enter"),
        KeyCode::Tab => state.send_key("Tab"),
        KeyCode::BackTab => state.send_key("BTab"),
        KeyCode::Backspace => state.send_key("BSpace"),
        KeyCode::Esc => state.send_key("Escape"),
        KeyCode::Up => state.send_key("Up"),
        KeyCode::Down => state.send_key("Down"),
        KeyCode::Left => state.send_key("Left"),
        KeyCode::Right => state.send_key("Right"),
        KeyCode::Home => state.send_key("Home"),
        KeyCode::End => state.send_key("End"),
        KeyCode::PageUp => state.send_key("PPage"),
        KeyCode::PageDown => state.send_key("NPage"),
        KeyCode::Delete => state.send_key("Delete"),
        KeyCode::Insert => state.send_key("Insert"),
        KeyCode::F(number) => state.send_key(&format!("F{number}")),
        _ => {}
    }
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &SessionViewState,
) -> Result<SessionHitboxes> {
    let mut hitboxes = SessionHitboxes::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            let [body, footer] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

            frame.render_widget(Clear, body);

            if state.panes.is_empty() {
                frame.render_widget(render_empty_state(state), body);
            } else {
                let mut pane_areas = Vec::new();
                for pane in &state.panes {
                    let area = pane_area(body, pane, &state.panes);
                    let panel = pane_panel_area(body, area);
                    pane_areas.push(panel);
                    hitboxes.panes.push(PaneHitbox {
                        pane_id: pane.id.clone(),
                        area: panel,
                    });
                    render_pane(frame, state, pane, area);
                }
                render_pane_panels(frame, &pane_areas, state.selected_pane);
                if let Some(cursor) = selected_cursor_position(body, state) {
                    frame.set_cursor_position(cursor);
                }
            }

            frame.render_widget(
                Paragraph::new(footer_text(state)).style(Style::default().fg(Color::DarkGray)),
                footer,
            );
        })
        .context("failed to draw session frame")?;
    Ok(hitboxes)
}

fn render_empty_state(state: &SessionViewState) -> Paragraph<'static> {
    Paragraph::new(vec![
        Line::from(Span::styled(
            state.session.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(state.message.clone()),
    ])
    .block(
        Block::default()
            .title(" session ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
}

fn render_pane(frame: &mut ratatui::Frame<'_>, state: &SessionViewState, pane: &Pane, area: Rect) {
    let lines = state
        .captures
        .get(&pane.id)
        .cloned()
        .unwrap_or_else(|| vec![Line::from("")]);

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_pane_panels(frame: &mut ratatui::Frame<'_>, panels: &[Rect], selected_pane: usize) {
    for (index, panel) in panels.iter().copied().enumerate() {
        let style = if index == selected_pane {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        render_panel_border(frame, panel, style);
    }
}

fn render_panel_border(frame: &mut ratatui::Frame<'_>, panel: Rect, style: Style) {
    if panel.width < 2 || panel.height < 2 {
        return;
    }

    let x_end = panel.x.saturating_add(panel.width);
    let y_end = panel.y.saturating_add(panel.height);
    for y in panel.y..y_end {
        for x in panel.x..x_end {
            if let Some(symbol) = panel_border_symbol_at(x, y, panel) {
                draw_cell(frame, x, y, symbol, style);
            }
        }
    }
}

fn panel_border_symbol_at(x: u16, y: u16, panel: Rect) -> Option<&'static str> {
    if panel.width < 2 || panel.height < 2 {
        return None;
    }

    let left = panel.x;
    let right = panel.x + panel.width - 1;
    let top = panel.y;
    let bottom = panel.y + panel.height - 1;

    match (x, y) {
        (x, y) if x == left && y == top => Some("╭"),
        (x, y) if x == right && y == top => Some("╮"),
        (x, y) if x == left && y == bottom => Some("╰"),
        (x, y) if x == right && y == bottom => Some("╯"),
        (_, y) if y == top || y == bottom => Some("─"),
        (x, _) if x == left || x == right => Some("│"),
        _ => None,
    }
}

fn draw_cell(frame: &mut ratatui::Frame<'_>, x: u16, y: u16, symbol: &'static str, style: Style) {
    frame.buffer_mut()[(x, y)]
        .set_symbol(symbol)
        .set_style(style);
}

fn separator_symbol_at(x: u16, y: u16, panes: &[Rect]) -> Option<&'static str> {
    if panes.iter().any(|pane| rect_contains(*pane, x, y)) {
        return None;
    }

    let left = x > 0 && point_in_any_pane(x - 1, y, panes);
    let right = point_in_any_pane(x.saturating_add(1), y, panes);
    let up = y > 0 && point_in_any_pane(x, y - 1, panes);
    let down = point_in_any_pane(x, y.saturating_add(1), panes);
    let up_left = x > 0 && y > 0 && point_in_any_pane(x - 1, y - 1, panes);
    let up_right = y > 0 && point_in_any_pane(x.saturating_add(1), y - 1, panes);
    let down_left = x > 0 && point_in_any_pane(x - 1, y.saturating_add(1), panes);
    let down_right = point_in_any_pane(x.saturating_add(1), y.saturating_add(1), panes);

    let vertical = left && right;
    let horizontal = up && down;
    if vertical || horizontal {
        return match (vertical, horizontal) {
            (true, true) => Some("┼"),
            (true, false) => Some("│"),
            (false, true) => Some("─"),
            (false, false) => None,
        };
    }

    if up_left && up_right && down_left && down_right {
        Some("┼")
    } else {
        None
    }
}

fn point_in_any_pane(x: u16, y: u16, panes: &[Rect]) -> bool {
    panes.iter().any(|pane| rect_contains(*pane, x, y))
}

fn selected_cursor_position(body: Rect, state: &SessionViewState) -> Option<(u16, u16)> {
    if state.command_mode {
        return None;
    }

    let pane = state.panes.get(state.selected_pane)?;
    if !pane.cursor_visible || state.scroll_offset(&pane.id) > 0 {
        return None;
    }

    let area = pane_area(body, pane, &state.panes);
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let x = (pane.cursor_x as u16).min(area.width.saturating_sub(1));
    let y = (pane.cursor_y as u16).min(area.height.saturating_sub(1));
    Some((area.x + x, area.y + y))
}

fn footer_text(state: &SessionViewState) -> String {
    if state.command_mode {
        return " COMMAND | v split right | h split down | x close | r refresh | q detach | Esc cancel ".to_owned();
    }

    let selected = state
        .selected_pane_id()
        .unwrap_or_else(|| "no pane".to_owned());
    format!(
        " {} | {} | Ctrl-A commands | click pane | wheel scroll ",
        selected, state.message
    )
}

fn body_area(area: Rect) -> Rect {
    let [body, _footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
    body
}

fn tmux_content_size_for_body(body: Rect, panes: &[Pane]) -> (u16, u16) {
    let pane_rects = tmux_pane_rects(panes);
    let columns = separator_columns(&pane_rects);
    let rows = separator_rows(&pane_rects);

    let width = body
        .width
        .saturating_sub(2)
        .saturating_sub(columns.len() as u16)
        .max(1);
    let height = body
        .height
        .saturating_sub(2)
        .saturating_sub(rows.len() as u16)
        .max(1);
    (width, height)
}

fn tmux_pane_rects(panes: &[Pane]) -> Vec<Rect> {
    panes
        .iter()
        .map(|pane| Rect {
            x: pane.left.min(u16::MAX as usize) as u16,
            y: pane.top.min(u16::MAX as usize) as u16,
            width: pane.width.min(u16::MAX as usize) as u16,
            height: pane.height.min(u16::MAX as usize) as u16,
        })
        .collect()
}

fn separator_columns(panes: &[Rect]) -> Vec<u16> {
    let Some((max_x, max_y)) = layout_bounds(panes) else {
        return Vec::new();
    };
    let mut columns = Vec::new();
    for x in 0..max_x {
        let has_separator =
            (0..max_y).any(|y| matches!(separator_symbol_at(x, y, panes), Some("│") | Some("┼")));
        if has_separator {
            columns.push(x);
        }
    }
    columns
}

fn separator_rows(panes: &[Rect]) -> Vec<u16> {
    let Some((max_x, max_y)) = layout_bounds(panes) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for y in 0..max_y {
        let has_separator =
            (0..max_x).any(|x| matches!(separator_symbol_at(x, y, panes), Some("─") | Some("┼")));
        if has_separator {
            rows.push(y);
        }
    }
    rows
}

fn layout_bounds(panes: &[Rect]) -> Option<(u16, u16)> {
    let max_x = panes
        .iter()
        .map(|pane| pane.x.saturating_add(pane.width))
        .max()?;
    let max_y = panes
        .iter()
        .map(|pane| pane.y.saturating_add(pane.height))
        .max()?;
    Some((max_x, max_y))
}

fn count_before(value: u16, sorted_values: &[u16]) -> u16 {
    sorted_values
        .iter()
        .take_while(|position| **position < value)
        .count()
        .min(u16::MAX as usize) as u16
}

fn pane_area(body: Rect, pane: &Pane, panes: &[Pane]) -> Rect {
    let available_width = body.width.saturating_sub(2);
    let available_height = body.height.saturating_sub(2);
    if available_width == 0 || available_height == 0 {
        return body;
    }

    let pane_rects = tmux_pane_rects(panes);
    let columns = separator_columns(&pane_rects);
    let rows = separator_rows(&pane_rects);
    let pane_left = pane.left.min(u16::MAX as usize) as u16;
    let pane_top = pane.top.min(u16::MAX as usize) as u16;
    let pane_width = pane.width.min(u16::MAX as usize) as u16;
    let pane_height = pane.height.min(u16::MAX as usize) as u16;
    let relative_x = pane_left
        .saturating_add(count_before(pane_left, &columns))
        .min(available_width - 1);
    let relative_y = pane_top
        .saturating_add(count_before(pane_top, &rows))
        .min(available_height - 1);
    let max_width = available_width.saturating_sub(relative_x);
    let max_height = available_height.saturating_sub(relative_y);
    let display_width = pane_width.saturating_add(count_inside(
        pane_left,
        pane_left.saturating_add(pane_width),
        &columns,
    ));
    let display_height = pane_height.saturating_add(count_inside(
        pane_top,
        pane_top.saturating_add(pane_height),
        &rows,
    ));

    Rect {
        x: body.x + 1 + relative_x,
        y: body.y + 1 + relative_y,
        width: display_width.min(max_width).max(1),
        height: display_height.min(max_height).max(1),
    }
}

fn count_inside(start: u16, end: u16, sorted_values: &[u16]) -> u16 {
    sorted_values
        .iter()
        .filter(|position| **position >= start && **position < end)
        .count()
        .min(u16::MAX as usize) as u16
}

fn pane_panel_area(body: Rect, content: Rect) -> Rect {
    if body.width == 0 || body.height == 0 {
        return body;
    }

    let x = content.x.saturating_sub(1).max(body.x);
    let y = content.y.saturating_sub(1).max(body.y);
    let right = content
        .x
        .saturating_add(content.width)
        .saturating_add(1)
        .min(body.x.saturating_add(body.width));
    let bottom = content
        .y
        .saturating_add(content.height)
        .saturating_add(1)
        .min(body.y.saturating_add(body.height));

    Rect {
        x,
        y,
        width: right.saturating_sub(x),
        height: bottom.saturating_sub(y),
    }
}

fn capture_to_lines(capture: &str) -> Vec<Line<'static>> {
    let mut parser = AnsiLineParser::default();
    parser.push_capture(capture);
    parser.finish()
}

#[derive(Debug, Default)]
struct AnsiLineParser {
    lines: Vec<Vec<Span<'static>>>,
    buffer: String,
    style: Style,
}

impl AnsiLineParser {
    fn push_capture(&mut self, capture: &str) {
        self.lines.push(Vec::new());

        let mut chars = capture.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\x1b' => self.handle_escape(&mut chars),
                '\n' => {
                    self.flush_buffer();
                    self.lines.push(Vec::new());
                }
                '\r' => {}
                ch => self.buffer.push(ch),
            }
        }
        self.flush_buffer();

        if capture.ends_with('\n') && self.lines.last().is_some_and(Vec::is_empty) {
            self.lines.pop();
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if self.lines.is_empty() {
            self.lines.push(Vec::new());
        }

        self.lines
            .into_iter()
            .map(|spans| {
                if spans.is_empty() {
                    Line::from("")
                } else {
                    Line::from(spans)
                }
            })
            .collect()
    }

    fn handle_escape<I>(&mut self, chars: &mut std::iter::Peekable<I>)
    where
        I: Iterator<Item = char>,
    {
        let Some(prefix) = chars.next() else {
            return;
        };

        match prefix {
            '[' => self.handle_csi(chars),
            ']' | 'P' | 'X' | '^' | '_' => skip_string_escape(chars),
            '(' | ')' | '*' | '+' | '-' | '.' | '/' | '#' | '%' => {
                chars.next();
            }
            _ => {}
        }
    }

    fn handle_csi<I>(&mut self, chars: &mut std::iter::Peekable<I>)
    where
        I: Iterator<Item = char>,
    {
        let mut sequence = String::new();
        for ch in chars.by_ref() {
            if ('\u{40}'..='\u{7e}').contains(&ch) {
                if ch == 'm' {
                    self.flush_buffer();
                    apply_sgr(&mut self.style, &sequence);
                }
                return;
            }
            sequence.push(ch);
        }
    }

    fn flush_buffer(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        if let Some(line) = self.lines.last_mut() {
            line.push(Span::styled(std::mem::take(&mut self.buffer), self.style));
        }
    }
}

fn skip_string_escape<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    while let Some(ch) = chars.next() {
        match ch {
            '\x07' | '\u{9c}' => return,
            '\x1b' if chars.next_if_eq(&'\\').is_some() => return,
            _ => {}
        }
    }
}

fn apply_sgr(style: &mut Style, sequence: &str) {
    let params = parse_sgr_params(sequence);
    let mut index = 0;

    while index < params.len() {
        let value = params[index];
        match value {
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            2 => *style = style.add_modifier(Modifier::DIM),
            3 => *style = style.add_modifier(Modifier::ITALIC),
            4 => *style = style.add_modifier(Modifier::UNDERLINED),
            22 => *style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => *style = style.remove_modifier(Modifier::ITALIC),
            24 => *style = style.remove_modifier(Modifier::UNDERLINED),
            30..=37 => style.fg = basic_color(value - 30),
            39 => style.fg = None,
            40..=47 => style.bg = basic_color(value - 40),
            49 => style.bg = None,
            90..=97 => style.fg = bright_color(value - 90),
            100..=107 => style.bg = bright_color(value - 100),
            38 | 48 => {
                let target_is_fg = value == 38;
                if let Some((color, consumed)) = parse_extended_color(&params[index + 1..]) {
                    if target_is_fg {
                        style.fg = Some(color);
                    } else {
                        style.bg = Some(color);
                    }
                    index += consumed;
                }
            }
            _ => {}
        }
        index += 1;
    }
}

fn parse_sgr_params(sequence: &str) -> Vec<u16> {
    if sequence.trim().is_empty() {
        return vec![0];
    }

    sequence
        .split(';')
        .map(|part| {
            if part.is_empty() {
                0
            } else {
                part.parse::<u16>().unwrap_or(u16::MAX)
            }
        })
        .collect()
}

fn basic_color(index: u16) -> Option<Color> {
    Some(match index {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        _ => return None,
    })
}

fn bright_color(index: u16) -> Option<Color> {
    Some(match index {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        7 => Color::White,
        _ => return None,
    })
}

fn parse_extended_color(params: &[u16]) -> Option<(Color, usize)> {
    match params {
        [5, index, ..] => Some((Color::Indexed((*index).min(u8::MAX as u16) as u8), 2)),
        [2, red, green, blue, ..] => Some((
            Color::Rgb(
                (*red).min(u8::MAX as u16) as u8,
                (*green).min(u8::MAX as u16) as u8,
                (*blue).min(u8::MAX as u16) as u8,
            ),
            4,
        )),
        _ => None,
    }
}

fn hit_pane(hitboxes: &[PaneHitbox], column: u16, row: u16) -> Option<&str> {
    hitboxes
        .iter()
        .find(|hitbox| rect_contains(hitbox.area, column, row))
        .map(|hitbox| hitbox.pane_id.as_str())
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn select_pane_index(panes: &[Pane], preferred_id: Option<&str>, fallback: usize) -> usize {
    if panes.is_empty() {
        return 0;
    }

    if let Some(preferred_id) = preferred_id
        && let Some(index) = panes.iter().position(|pane| pane.id == preferred_id)
    {
        return index;
    }

    if let Some(index) = panes.iter().position(|pane| pane.active) {
        return index;
    }

    fallback.min(panes.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_capture_preserves_basic_foreground_color() {
        let lines = capture_to_lines("\x1b[31mred\x1b[39m");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 1);
        assert_eq!(lines[0].spans[0].content.as_ref(), "red");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn ansi_capture_preserves_256_color_and_reset() {
        let lines = capture_to_lines("\x1b[38;5;196mhot\x1b[0m plain");

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "hot");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Indexed(196)));
        assert_eq!(lines[0].spans[1].content.as_ref(), " plain");
        assert_eq!(lines[0].spans[1].style.fg, None);
    }

    #[test]
    fn ansi_capture_keeps_blank_terminal_lines_without_extra_final_line() {
        let lines = capture_to_lines("one\n\n");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content.as_ref(), "one");
        assert!(lines[1].spans.is_empty());
    }

    #[test]
    fn ansi_capture_skips_osc_title_escape() {
        let lines = capture_to_lines("before\x1b]0;ignored title\x07 after");

        assert_eq!(line_text(&lines[0]), "before after");
    }

    #[test]
    fn ansi_capture_skips_osc_8_hyperlink_escape() {
        let lines =
            capture_to_lines("open \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ done");

        assert_eq!(line_text(&lines[0]), "open link done");
    }

    #[test]
    fn ansi_capture_skips_non_sgr_csi_with_symbol_final_byte() {
        let lines = capture_to_lines("a\x1b[200~pasted\x1b[201~z");

        assert_eq!(line_text(&lines[0]), "apastedz");
    }

    #[test]
    fn cursor_position_offsets_with_panel_border_and_keeps_content_geometry() {
        let mut state = SessionViewState::new("work");
        state.panes = vec![Pane {
            id: "%1".to_owned(),
            index: 0,
            active: true,
            current_command: "bash".to_owned(),
            current_path: "/tmp".to_owned(),
            width: 78,
            height: 22,
            left: 0,
            top: 0,
            cursor_x: 37,
            cursor_y: 3,
            cursor_visible: true,
        }];

        let cursor = selected_cursor_position(
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            &state,
        );

        assert_eq!(cursor, Some((38, 4)));
    }

    #[test]
    fn cursor_position_respects_split_pane_origin() {
        let mut state = SessionViewState::new("work");
        state.panes = vec![
            Pane {
                id: "%1".to_owned(),
                index: 0,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 39,
                height: 22,
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
                width: 38,
                height: 22,
                left: 40,
                top: 0,
                cursor_x: 5,
                cursor_y: 2,
                cursor_visible: true,
            },
        ];
        state.selected_pane = 1;

        let cursor = selected_cursor_position(
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            &state,
        );

        assert_eq!(cursor, Some((47, 3)));
    }

    #[test]
    fn pane_area_adds_visual_gap_for_tmux_separator_column() {
        let panes = vec![
            Pane {
                id: "%1".to_owned(),
                index: 0,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 39,
                height: 22,
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
                width: 38,
                height: 22,
                left: 40,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
        ];

        let body = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let left = pane_area(body, &panes[0], &panes);
        let right = pane_area(body, &panes[1], &panes);

        assert_eq!(left, Rect::new(1, 1, 39, 22));
        assert_eq!(right, Rect::new(42, 1, 37, 22));
    }

    #[test]
    fn pane_area_expands_panes_that_span_adjacent_split_rows() {
        let panes = vec![
            Pane {
                id: "%1".to_owned(),
                index: 0,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 38,
                height: 20,
                left: 0,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
            Pane {
                id: "%2".to_owned(),
                index: 1,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 38,
                height: 9,
                left: 39,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
            Pane {
                id: "%3".to_owned(),
                index: 2,
                active: true,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 38,
                height: 10,
                left: 39,
                top: 10,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
        ];

        let body = Rect::new(0, 0, 80, 23);
        let left = pane_area(body, &panes[0], &panes);
        let top_right = pane_area(body, &panes[1], &panes);
        let bottom_right = pane_area(body, &panes[2], &panes);

        assert_eq!(left, Rect::new(1, 1, 38, 21));
        assert_eq!(top_right, Rect::new(41, 1, 38, 9));
        assert_eq!(bottom_right, Rect::new(41, 12, 38, 10));
    }

    #[test]
    fn separator_uses_gap_column_between_side_by_side_panes() {
        let panes = [
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 24,
            },
            Rect {
                x: 41,
                y: 0,
                width: 39,
                height: 24,
            },
        ];

        assert_eq!(separator_symbol_at(39, 4, &panes), None);
        assert_eq!(separator_symbol_at(40, 4, &panes), Some("│"));
        assert_eq!(separator_symbol_at(41, 4, &panes), None);
    }

    #[test]
    fn separator_uses_gap_row_between_stacked_panes() {
        let panes = [
            Rect {
                x: 0,
                y: 0,
                width: 80,
                height: 10,
            },
            Rect {
                x: 0,
                y: 11,
                width: 80,
                height: 13,
            },
        ];

        assert_eq!(separator_symbol_at(12, 9, &panes), None);
        assert_eq!(separator_symbol_at(12, 10, &panes), Some("─"));
        assert_eq!(separator_symbol_at(12, 11, &panes), None);
    }

    #[test]
    fn separator_marks_crossing_between_four_panes() {
        let panes = [
            Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 10,
            },
            Rect {
                x: 41,
                y: 0,
                width: 39,
                height: 10,
            },
            Rect {
                x: 0,
                y: 11,
                width: 40,
                height: 13,
            },
            Rect {
                x: 41,
                y: 11,
                width: 39,
                height: 13,
            },
        ];

        assert_eq!(separator_symbol_at(40, 10, &panes), Some("┼"));
    }

    #[test]
    fn panel_area_wraps_each_pane_as_an_independent_box() {
        let body = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        let left = pane_panel_area(body, Rect::new(1, 1, 39, 22));
        let right = pane_panel_area(body, Rect::new(42, 1, 37, 22));

        assert_eq!(left, Rect::new(0, 0, 41, 24));
        assert_eq!(right, Rect::new(41, 0, 39, 24));
        assert_eq!(panel_border_symbol_at(0, 0, left), Some("╭"));
        assert_eq!(panel_border_symbol_at(40, 0, left), Some("╮"));
        assert_eq!(panel_border_symbol_at(41, 0, right), Some("╭"));
        assert_eq!(panel_border_symbol_at(79, 23, right), Some("╯"));
    }

    #[test]
    fn tmux_content_size_reserves_cells_for_independent_panel_chrome() {
        let panes = vec![
            Pane {
                id: "%1".to_owned(),
                index: 0,
                active: false,
                current_command: "bash".to_owned(),
                current_path: "/tmp".to_owned(),
                width: 39,
                height: 22,
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
                width: 38,
                height: 22,
                left: 40,
                top: 0,
                cursor_x: 0,
                cursor_y: 0,
                cursor_visible: true,
            },
        ];

        assert_eq!(
            tmux_content_size_for_body(Rect::new(0, 0, 80, 24), &panes),
            (77, 22)
        );
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
