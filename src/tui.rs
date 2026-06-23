use std::fs;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::history::Conversation;
use crate::{render, search};

pub enum Action {
    Quit,
    Select(Conversation),
    Resume(Conversation),
    Fork(Conversation),
}

pub struct TuiInput {
    pub conversations: Vec<Conversation>,
    pub initial_query: String,
    pub local_filter: bool,
    pub current_dir: PathBuf,
    pub show_tools: bool,
    pub show_reasoning: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Viewer,
    Help,
    ConfirmDelete,
}

struct App {
    conversations: Vec<Conversation>,
    filtered: Vec<usize>,
    current_dir: PathBuf,
    query: String,
    local_filter: bool,
    last_filter_query: String,
    last_filter_local_filter: bool,
    selected: usize,
    list_offset: usize,
    mode: Mode,
    viewer_index: Option<usize>,
    viewer_scroll: usize,
    viewer_lines: Vec<render::TuiLine>,
    viewer_render_key: Option<(usize, u16, render::ToolDisplay, bool)>,
    viewer_search: String,
    entering_viewer_search: bool,
    tool_display: render::ToolDisplay,
    show_reasoning: bool,
    status_message: Option<String>,
    delete_target_index: Option<usize>,
    delete_return_mode: Mode,
}

pub fn run(input: TuiInput) -> Result<Action> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run_app(&mut terminal, input);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, input: TuiInput) -> Result<Action> {
    let mut app = App {
        conversations: input.conversations,
        filtered: Vec::new(),
        current_dir: input.current_dir,
        query: input.initial_query,
        local_filter: input.local_filter,
        last_filter_query: String::new(),
        last_filter_local_filter: input.local_filter,
        selected: 0,
        list_offset: 0,
        mode: Mode::List,
        viewer_index: None,
        viewer_scroll: 0,
        viewer_lines: Vec::new(),
        viewer_render_key: None,
        viewer_search: String::new(),
        entering_viewer_search: false,
        tool_display: render::ToolDisplay::from_show_tools(input.show_tools),
        show_reasoning: input.show_reasoning,
        status_message: None,
        delete_target_index: None,
        delete_return_mode: Mode::List,
    };
    app.refresh_filter();

    loop {
        terminal.draw(|frame| draw(frame, &mut app))?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => match app.mode {
                Mode::List => {
                    if let Some(action) = handle_list_key(&mut app, key) {
                        return Ok(action);
                    }
                }
                Mode::Viewer => {
                    if let Some(action) = handle_viewer_key(&mut app, key) {
                        return Ok(action);
                    }
                }
                Mode::Help => handle_help_key(&mut app, key),
                Mode::ConfirmDelete => handle_confirm_delete_key(&mut app, key),
            },
            Event::Mouse(mouse) => match app.mode {
                Mode::List => match mouse.kind {
                    MouseEventKind::ScrollDown => app.move_selection(1),
                    MouseEventKind::ScrollUp => app.move_selection(-1),
                    _ => {}
                },
                Mode::Viewer => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        app.viewer_scroll = app.viewer_scroll.saturating_add(3)
                    }
                    MouseEventKind::ScrollUp => {
                        app.viewer_scroll = app.viewer_scroll.saturating_sub(3)
                    }
                    _ => {}
                },
                Mode::Help => {}
                Mode::ConfirmDelete => {}
            },
            _ => {}
        }
    }
}

fn handle_list_key(app: &mut App, key: KeyEvent) -> Option<Action> {
    match (key.modifiers, key.code) {
        (_, KeyCode::Char('?')) => app.mode = Mode::Help,
        (_, KeyCode::Char('D')) | (_, KeyCode::Delete) => {
            app.confirm_delete_selected();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Some(Action::Quit),
        (_, KeyCode::Esc) => {
            if app.query.is_empty() {
                return Some(Action::Quit);
            }
            app.query.clear();
            app.refresh_filter();
        }
        (_, KeyCode::Char(ch))
            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
        {
            app.query.push(ch);
            app.refresh_filter();
        }
        (_, KeyCode::Backspace) => {
            app.query.pop();
            app.refresh_filter();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            trim_last_word(&mut app.query);
            app.refresh_filter();
        }
        (_, KeyCode::Down) | (KeyModifiers::CONTROL, KeyCode::Char('n')) => app.move_selection(1),
        (_, KeyCode::Up) | (KeyModifiers::CONTROL, KeyCode::Char('p')) => app.move_selection(-1),
        (_, KeyCode::PageDown) | (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            app.move_selection(10)
        }
        (_, KeyCode::PageUp) | (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.move_selection(-10)
        }
        (_, KeyCode::Home) => app.selected = 0,
        (_, KeyCode::End) => app.selected = app.filtered.len().saturating_sub(1),
        (_, KeyCode::Tab) => {
            app.local_filter = !app.local_filter;
            app.refresh_filter();
        }
        (_, KeyCode::Enter) => app.open_viewer(),
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            return app.selected_conversation().cloned().map(Action::Select);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
            return app.selected_conversation().cloned().map(Action::Resume);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            return app.selected_conversation().cloned().map(Action::Fork);
        }
        _ => {}
    }
    None
}

fn handle_confirm_delete_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.delete_target_session();
            app.mode = Mode::List;
        }
        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
            app.delete_target_index = None;
            app.mode = app.delete_return_mode;
        }
        _ => {}
    }
}

fn handle_viewer_key(app: &mut App, key: KeyEvent) -> Option<Action> {
    if app.entering_viewer_search {
        match key.code {
            KeyCode::Esc => app.entering_viewer_search = false,
            KeyCode::Enter => {
                app.entering_viewer_search = false;
                app.jump_to_viewer_match(true);
            }
            KeyCode::Backspace => {
                app.viewer_search.pop();
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                app.viewer_search.push(ch);
            }
            _ => {}
        }
        return None;
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Some(Action::Quit),
        (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => app.mode = Mode::List,
        (_, KeyCode::Char('?')) => app.mode = Mode::Help,
        (_, KeyCode::Char('D')) | (_, KeyCode::Delete) => {
            app.confirm_delete_viewer();
        }
        (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
            app.viewer_scroll = app.viewer_scroll.saturating_add(1)
        }
        (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
            app.viewer_scroll = app.viewer_scroll.saturating_sub(1)
        }
        (_, KeyCode::PageDown)
        | (KeyModifiers::CONTROL, KeyCode::Char('d'))
        | (_, KeyCode::Char('d')) => app.viewer_scroll = app.viewer_scroll.saturating_add(20),
        (_, KeyCode::PageUp)
        | (KeyModifiers::CONTROL, KeyCode::Char('u'))
        | (_, KeyCode::Char('u')) => app.viewer_scroll = app.viewer_scroll.saturating_sub(20),
        (_, KeyCode::Home) | (_, KeyCode::Char('g')) => app.viewer_scroll = 0,
        (_, KeyCode::End) | (_, KeyCode::Char('G')) => {
            app.viewer_scroll = app.viewer_lines.len().saturating_sub(1)
        }
        (_, KeyCode::Char('/')) => {
            app.viewer_search.clear();
            app.entering_viewer_search = true;
        }
        (_, KeyCode::Char('n')) => app.jump_to_viewer_match(true),
        (_, KeyCode::Char('N')) => app.jump_to_viewer_match(false),
        (_, KeyCode::Char('t')) => {
            app.tool_display = app.tool_display.next();
            app.invalidate_viewer();
        }
        (_, KeyCode::Char('T')) => {
            app.show_reasoning = !app.show_reasoning;
            app.invalidate_viewer();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
            return app.viewer_conversation().cloned().map(Action::Resume);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            return app.viewer_conversation().cloned().map(Action::Fork);
        }
        _ => {}
    }
    None
}

fn handle_help_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            app.mode = if app.viewer_index.is_some() {
                Mode::Viewer
            } else {
                Mode::List
            };
        }
        _ => {}
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    match app.mode {
        Mode::List => draw_list(frame, app),
        Mode::Viewer => draw_viewer(frame, app),
        Mode::Help => draw_help(frame, app),
        Mode::ConfirmDelete => {
            if app.delete_return_mode == Mode::Viewer {
                draw_viewer(frame, app);
            } else {
                draw_list(frame, app);
            }
            draw_delete_confirm(frame, app);
        }
    }
}

fn draw_list(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);
    let scope = if app.local_filter { "local" } else { "all" };
    let input_width = chunks[0].width.saturating_sub(2);
    let visible_query = visible_input_tail(&app.query, input_width);
    let cursor_offset = UnicodeWidthStr::width(visible_query.as_str()) as u16;
    let input = Paragraph::new(visible_query).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" codex-history [{scope}] ")),
    );
    frame.render_widget(input, chunks[0]);
    if input_width > 0 {
        frame.set_cursor_position((
            chunks[0].x + 1 + cursor_offset.min(input_width.saturating_sub(1)),
            chunks[0].y + 1,
        ));
    }

    let visible_height = usize::from(chunks[1].height.saturating_sub(2)).max(1);
    app.ensure_selection_visible(visible_height);
    let items = app
        .filtered
        .iter()
        .skip(app.list_offset)
        .take(visible_height)
        .enumerate()
        .map(|(row, &idx)| {
            let conversation = &app.conversations[idx];
            let is_selected = app.list_offset + row == app.selected;
            let metadata_style = list_metadata_style(is_selected);
            let cwd_style = if is_selected {
                Style::default().fg(Color::Rgb(128, 210, 255))
            } else {
                Style::default().fg(Color::Blue)
            };
            let preview_style = if is_selected {
                Style::default().fg(Color::Rgb(220, 220, 220))
            } else {
                Style::default().fg(Color::Gray)
            };
            let cwd = conversation
                .cwd
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unknown cwd)".to_string());
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    conversation.started_at.format("%Y-%m-%d %H:%M").to_string(),
                    metadata_style,
                ),
                Span::raw("  "),
                Span::styled(
                    &conversation.title,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ])];
            lines.push(Line::from(vec![
                Span::styled(cwd, cwd_style),
                Span::raw("  "),
                Span::styled(&conversation.session_id, metadata_style),
            ]));
            if !conversation.preview.is_empty() {
                lines.push(Line::from(Span::styled(
                    &conversation.preview,
                    preview_style,
                )));
            }
            ListItem::new(lines)
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(app.selected.checked_sub(app.list_offset));
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} sessions ", app.filtered.len())),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut state);

    let footer = app.status_message.as_deref().unwrap_or(
        "type to search | Enter open | Tab scope | D delete | Ctrl+R resume | Ctrl+F fork | ? help | Esc quit",
    );
    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

fn draw_viewer(frame: &mut ratatui::Frame, app: &mut App) {
    let area = frame.area();
    app.ensure_viewer_rendered(area.width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let max_scroll = app
        .viewer_lines
        .len()
        .saturating_sub(usize::from(chunks[0].height).saturating_sub(2));
    app.viewer_scroll = app.viewer_scroll.min(max_scroll);
    let text = app
        .viewer_lines
        .iter()
        .skip(app.viewer_scroll)
        .map(|line| render::highlighted_line(line, &app.viewer_search))
        .collect::<Vec<_>>();
    let title = app
        .viewer_conversation()
        .map(|c| format!(" {} ", c.title))
        .unwrap_or_else(|| " conversation ".to_string());
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 60)))
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(Color::Rgb(78, 201, 176))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[0]);

    let footer = if app.entering_viewer_search {
        Line::from(vec![
            Span::styled("search  ", Style::default().fg(Color::Rgb(100, 100, 100))),
            Span::styled(
                format!("/{}", app.viewer_search),
                Style::default().fg(Color::White),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k", key_style()),
            Span::raw(" scroll  "),
            Span::styled("/", key_style()),
            Span::raw(" search  "),
            Span::styled("n/N", key_style()),
            Span::raw(" next  "),
            Span::styled("t", key_style()),
            Span::raw(format!(" tools:{}  ", app.tool_display.label())),
            Span::styled("T", key_style()),
            Span::raw(format!(
                " reasoning:{}  ",
                if app.show_reasoning { "on" } else { "off" }
            )),
            Span::styled("Ctrl+R", key_style()),
            Span::raw(" resume  "),
            Span::styled("D", key_style()),
            Span::raw(" delete  "),
            Span::styled("q", key_style()),
            Span::raw(" back"),
        ])
    };
    frame.render_widget(
        Paragraph::new(footer).style(
            Style::default()
                .fg(Color::Rgb(140, 140, 140))
                .bg(Color::Rgb(30, 30, 35)),
        ),
        chunks[1],
    );
}

fn draw_help(frame: &mut ratatui::Frame, _app: &App) {
    let area = centered_rect(72, 18, frame.area());
    let lines = vec![
        Line::from("List"),
        Line::from("  Type search text, Backspace edit, Ctrl+W delete word"),
        Line::from("  Up/Down or Ctrl+P/Ctrl+N move, PageUp/PageDown page"),
        Line::from("  Enter open, Tab toggle workspace scope"),
        Line::from("  D delete session, Ctrl+O select, Ctrl+R resume, Ctrl+F fork"),
        Line::from(""),
        Line::from("Viewer"),
        Line::from("  j/k scroll, g/G top/bottom, / search, n/N matches"),
        Line::from("  t cycle tools off/brief/on, T toggle reasoning"),
        Line::from("  D delete session, q return"),
        Line::from(""),
        Line::from("Esc, q, or ? closes this help"),
    ];
    let help = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" help "))
        .wrap(Wrap { trim: false });
    frame.render_widget(help, area);
}

fn draw_delete_confirm(frame: &mut ratatui::Frame, app: &App) {
    let area = centered_rect(76, 8, frame.area());
    let Some(conversation) = app.delete_target_conversation() else {
        return;
    };
    let text_width = usize::from(area.width.saturating_sub(2));
    let title = truncate_to_width(&conversation.title, text_width);
    let session_id = truncate_to_width(&conversation.session_id, text_width);
    let path = truncate_to_width(&conversation.path.display().to_string(), text_width);
    let lines = vec![
        Line::from(Span::styled(
            "Delete session?",
            Style::default()
                .fg(Color::Rgb(255, 180, 90))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(title),
        Line::from(Span::styled(
            session_id,
            Style::default().fg(Color::Rgb(210, 210, 210)),
        )),
        Line::from(Span::styled(
            path,
            Style::default().fg(Color::Rgb(150, 150, 150)),
        )),
        Line::from(""),
        Line::from("Press y to delete, Esc or n to cancel"),
    ];
    let confirm = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(255, 180, 90)))
                .title(" confirm delete "),
        )
        .style(Style::default().bg(Color::Rgb(25, 25, 28)));
    frame.render_widget(Clear, area);
    frame.render_widget(confirm, area);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height.min(area.height)),
            Constraint::Min((area.height.saturating_sub(height)) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min((area.width.saturating_sub(width)) / 2),
            Constraint::Length(width.min(area.width)),
            Constraint::Min((area.width.saturating_sub(width)) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

impl App {
    fn refresh_filter(&mut self) {
        let can_reuse_candidates = self.local_filter == self.last_filter_local_filter
            && !self.last_filter_query.is_empty()
            && self.query.starts_with(&self.last_filter_query);
        self.filtered = if can_reuse_candidates {
            search::filter_and_rank_candidate_indices(
                &self.conversations,
                self.filtered.clone(),
                &self.query,
                self.local_filter,
                &self.current_dir,
            )
        } else {
            search::filter_and_rank_indices(
                &self.conversations,
                &self.query,
                self.local_filter,
                &self.current_dir,
            )
        };
        self.last_filter_query = self.query.clone();
        self.last_filter_local_filter = self.local_filter;
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        self.list_offset = self.list_offset.min(self.selected);
    }

    fn selected_conversation(&self) -> Option<&Conversation> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.conversations.get(idx))
    }

    fn viewer_conversation(&self) -> Option<&Conversation> {
        self.viewer_index
            .and_then(|idx| self.conversations.get(idx))
    }

    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.filtered.len().saturating_sub(1));
    }

    fn ensure_selection_visible(&mut self, height: usize) {
        if self.selected < self.list_offset {
            self.list_offset = self.selected;
        } else if self.selected >= self.list_offset.saturating_add(height) {
            self.list_offset = self.selected.saturating_sub(height.saturating_sub(1));
        }
    }

    fn open_viewer(&mut self) {
        let Some(&idx) = self.filtered.get(self.selected) else {
            return;
        };
        self.viewer_index = Some(idx);
        self.viewer_scroll = 0;
        self.invalidate_viewer();
        self.mode = Mode::Viewer;
    }

    fn confirm_delete_selected(&mut self) {
        let Some(&idx) = self.filtered.get(self.selected) else {
            return;
        };
        self.status_message = None;
        self.delete_target_index = Some(idx);
        self.delete_return_mode = Mode::List;
        self.mode = Mode::ConfirmDelete;
    }

    fn confirm_delete_viewer(&mut self) {
        let Some(idx) = self.viewer_index else {
            return;
        };
        self.status_message = None;
        self.delete_target_index = Some(idx);
        self.delete_return_mode = Mode::Viewer;
        self.mode = Mode::ConfirmDelete;
    }

    fn delete_target_conversation(&self) -> Option<&Conversation> {
        self.delete_target_index
            .and_then(|idx| self.conversations.get(idx))
    }

    fn delete_target_session(&mut self) {
        let Some(idx) = self.delete_target_index.take() else {
            self.status_message = Some("no session selected".to_string());
            return;
        };
        if idx >= self.conversations.len() {
            self.status_message = Some("selected session no longer exists".to_string());
            return;
        }
        let path = self.conversations[idx].path.clone();
        let session_id = self.conversations[idx].session_id.clone();
        match fs::remove_file(&path) {
            Ok(()) => {
                self.conversations.remove(idx);
                self.viewer_index = None;
                self.viewer_scroll = 0;
                self.invalidate_viewer();
                self.last_filter_query.clear();
                self.refresh_filter();
                self.status_message = Some(format!("deleted session {session_id}"));
            }
            Err(err) => {
                self.status_message = Some(format!("failed to delete session {session_id}: {err}"));
            }
        }
    }

    fn invalidate_viewer(&mut self) {
        self.viewer_lines.clear();
        self.viewer_render_key = None;
    }

    fn ensure_viewer_rendered(&mut self, width: u16) {
        let Some(idx) = self.viewer_index else {
            self.invalidate_viewer();
            return;
        };
        let key = (idx, width, self.tool_display, self.show_reasoning);
        if self.viewer_render_key == Some(key) {
            return;
        }
        let conversation = &self.conversations[idx];
        self.viewer_lines =
            render::render_tui_lines(conversation, width, self.tool_display, self.show_reasoning);
        self.viewer_render_key = Some(key);
    }

    fn jump_to_viewer_match(&mut self, forward: bool) {
        let needle = self.viewer_search.to_ascii_lowercase();
        if needle.is_empty() {
            return;
        }
        let mut indices = self
            .viewer_lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| {
                line.text
                    .to_ascii_lowercase()
                    .contains(&needle)
                    .then_some(idx)
            })
            .collect::<Vec<_>>();
        if !forward {
            indices.reverse();
        }
        let target = indices.into_iter().find(|idx| {
            if forward {
                *idx > self.viewer_scroll
            } else {
                *idx < self.viewer_scroll
            }
        });
        if let Some(idx) = target {
            self.viewer_scroll = idx;
        }
    }
}

fn key_style() -> Style {
    Style::default()
        .fg(Color::Rgb(78, 201, 176))
        .add_modifier(Modifier::BOLD)
}

fn list_metadata_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(Color::Rgb(235, 235, 235))
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn trim_last_word(input: &mut String) {
    while input.ends_with(char::is_whitespace) {
        input.pop();
    }
    while input.chars().last().is_some_and(|ch| !ch.is_whitespace()) {
        input.pop();
    }
}

fn visible_input_tail(input: &str, max_width: u16) -> String {
    let max_width = usize::from(max_width);
    if max_width == 0 {
        return String::new();
    }

    let mut width = 0;
    let mut start = input.len();
    for (idx, ch) in input.char_indices().rev() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        width += ch_width;
        start = idx;
    }
    input[start..].to_string()
}

fn truncate_to_width(input: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(input) <= max_width {
        return input.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let mut output = String::new();
    let mut width = 0;
    let target_width = max_width - 3;
    for ch in input.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > target_width {
            break;
        }
        output.push(ch);
        width += ch_width;
    }
    output.push_str("...");
    output
}
