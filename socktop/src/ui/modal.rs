//! Modal window system for socktop TUI application

use super::theme::MODAL_DIM_BG;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

// Re-export types from modal_types
pub use super::modal_types::{
    ModalAction, ModalButton, ModalType, ProcessHistoryData, ProcessModalData,
};

#[derive(Debug)]
pub struct ModalManager {
    stack: Vec<ModalType>,
    pub(super) active_button: ModalButton,
    pub thread_scroll_offset: usize,
    pub journal_scroll_offset: usize,
    pub thread_scroll_max: usize,
    pub journal_scroll_max: usize,
}

impl ModalManager {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            active_button: ModalButton::Retry,
            thread_scroll_offset: 0,
            journal_scroll_offset: 0,
            thread_scroll_max: 0,
            journal_scroll_max: 0,
        }
    }
    pub fn is_active(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn current_modal(&self) -> Option<&ModalType> {
        self.stack.last()
    }

    pub fn push_modal(&mut self, modal: ModalType) {
        self.stack.push(modal);
        self.active_button = match self.stack.last() {
            Some(ModalType::ConnectionError { .. }) => ModalButton::Retry,
            Some(ModalType::ProcessDetails { .. }) => {
                // Reset scroll state for new process details
                self.thread_scroll_offset = 0;
                self.journal_scroll_offset = 0;
                self.thread_scroll_max = 0;
                self.journal_scroll_max = 0;
                ModalButton::Ok
            }
            Some(ModalType::About) => ModalButton::Ok,
            Some(ModalType::Help) => ModalButton::Ok,
            Some(ModalType::Confirmation { .. }) => ModalButton::Confirm,
            Some(ModalType::Info { .. }) => ModalButton::Ok,
            None => ModalButton::Ok,
        };
    }
    pub fn pop_modal(&mut self) -> Option<ModalType> {
        let m = self.stack.pop();
        if let Some(next) = self.stack.last() {
            self.active_button = match next {
                ModalType::ConnectionError { .. } => ModalButton::Retry,
                ModalType::ProcessDetails { .. } => ModalButton::Ok,
                ModalType::About => ModalButton::Ok,
                ModalType::Help => ModalButton::Ok,
                ModalType::Confirmation { .. } => ModalButton::Confirm,
                ModalType::Info { .. } => ModalButton::Ok,
            };
        }
        m
    }
    pub fn update_connection_error_countdown(&mut self, new_countdown: Option<u64>) {
        if let Some(ModalType::ConnectionError {
            auto_retry_countdown,
            ..
        }) = self.stack.last_mut()
        {
            *auto_retry_countdown = new_countdown;
        }
    }
    pub fn handle_key(&mut self, key: KeyCode) -> ModalAction {
        if !self.is_active() {
            return ModalAction::None;
        }
        match key {
            KeyCode::Esc => {
                self.pop_modal();
                ModalAction::Cancel
            }
            KeyCode::Enter => self.handle_enter(),
            KeyCode::Tab | KeyCode::Right => {
                self.next_button();
                ModalAction::None
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.prev_button();
                ModalAction::None
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if matches!(self.stack.last(), Some(ModalType::ConnectionError { .. })) {
                    ModalAction::RetryConnection
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                if matches!(self.stack.last(), Some(ModalType::ConnectionError { .. })) {
                    ModalAction::ExitApp
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    // Close all ProcessDetails modals at once (handles parent navigation chain)
                    while matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                        self.pop_modal();
                    }
                    ModalAction::Dismiss
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('j') | KeyCode::Char('J') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.thread_scroll_offset = self
                        .thread_scroll_offset
                        .saturating_add(1)
                        .min(self.thread_scroll_max);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.thread_scroll_offset = self.thread_scroll_offset.saturating_sub(1);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.thread_scroll_offset = self
                        .thread_scroll_offset
                        .saturating_add(10)
                        .min(self.thread_scroll_max);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.thread_scroll_offset = self.thread_scroll_offset.saturating_sub(10);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('[') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.journal_scroll_offset = self.journal_scroll_offset.saturating_sub(1);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char(']') => {
                if matches!(self.stack.last(), Some(ModalType::ProcessDetails { .. })) {
                    self.journal_scroll_offset = self
                        .journal_scroll_offset
                        .saturating_add(1)
                        .min(self.journal_scroll_max);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                // Switch to parent process if it exists
                if let Some(ModalType::ProcessDetails { pid }) = self.stack.last() {
                    // We need to get the parent PID from the process details
                    // For now, return a special action that the app can handle
                    // The app has access to the process details and can extract parent_pid
                    ModalAction::SwitchToParentProcess(*pid)
                } else {
                    ModalAction::None
                }
            }
            _ => ModalAction::None,
        }
    }
    fn handle_enter(&mut self) -> ModalAction {
        match (&self.stack.last(), &self.active_button) {
            (Some(ModalType::ConnectionError { .. }), ModalButton::Retry) => {
                ModalAction::RetryConnection
            }
            (Some(ModalType::ConnectionError { .. }), ModalButton::Exit) => ModalAction::ExitApp,
            (Some(ModalType::ProcessDetails { .. }), ModalButton::Ok) => {
                self.pop_modal();
                ModalAction::Dismiss
            }
            (Some(ModalType::About), ModalButton::Ok) => {
                self.pop_modal();
                ModalAction::Dismiss
            }
            (Some(ModalType::Help), ModalButton::Ok) => {
                self.pop_modal();
                ModalAction::Dismiss
            }
            (Some(ModalType::Confirmation { .. }), ModalButton::Confirm) => ModalAction::Confirm,
            (Some(ModalType::Confirmation { .. }), ModalButton::Cancel) => ModalAction::Cancel,
            (Some(ModalType::Info { .. }), ModalButton::Ok) => {
                self.pop_modal();
                ModalAction::Dismiss
            }
            _ => ModalAction::None,
        }
    }
    fn next_button(&mut self) {
        self.active_button = match (&self.stack.last(), &self.active_button) {
            (Some(ModalType::ConnectionError { .. }), ModalButton::Retry) => ModalButton::Exit,
            (Some(ModalType::ConnectionError { .. }), ModalButton::Exit) => ModalButton::Retry,
            (Some(ModalType::Confirmation { .. }), ModalButton::Confirm) => ModalButton::Cancel,
            (Some(ModalType::Confirmation { .. }), ModalButton::Cancel) => ModalButton::Confirm,
            _ => self.active_button.clone(),
        };
    }
    fn prev_button(&mut self) {
        self.next_button();
    }

    pub fn render(&mut self, f: &mut Frame, data: ProcessModalData) {
        if let Some(m) = self.stack.last().cloned() {
            self.render_background_dim(f);
            self.render_modal_content(f, &m, data);
        }
    }

    fn render_background_dim(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Clear, area);
        f.render_widget(
            Block::default()
                .style(Style::default().bg(MODAL_DIM_BG).fg(MODAL_DIM_BG))
                .borders(Borders::NONE),
            area,
        );
    }

    fn render_modal_content(&mut self, f: &mut Frame, modal: &ModalType, data: ProcessModalData) {
        let area = f.area();
        // Different sizes for different modal types
        let modal_area = match modal {
            ModalType::ProcessDetails { .. } => {
                // Process details modal uses almost full screen (95% width, 90% height)
                self.centered_rect(95, 90, area)
            }
            ModalType::About => {
                // About modal uses medium size
                self.centered_rect(90, 90, area)
            }
            ModalType::Help => {
                // Help modal uses medium size
                self.centered_rect(70, 80, area)
            }
            _ => {
                // Other modals use smaller size
                self.centered_rect(70, 50, area)
            }
        };
        f.render_widget(Clear, modal_area);
        match modal {
            ModalType::ConnectionError {
                message,
                disconnected_at,
                retry_count,
                auto_retry_countdown,
            } => self.render_connection_error(
                f,
                modal_area,
                message,
                *disconnected_at,
                *retry_count,
                *auto_retry_countdown,
            ),
            ModalType::ProcessDetails { pid } => {
                self.render_process_details(f, modal_area, *pid, data)
            }
            ModalType::About => self.render_about(f, modal_area),
            ModalType::Help => self.render_help(f, modal_area),
            ModalType::Confirmation {
                title,
                message,
                confirm_text,
                cancel_text,
            } => self.render_confirmation(f, modal_area, title, message, confirm_text, cancel_text),
            ModalType::Info { title, message } => self.render_info(f, modal_area, title, message),
        }
    }

    fn render_confirmation(
        &self,
        f: &mut Frame,
        area: Rect,
        title: &str,
        message: &str,
        confirm_text: &str,
        cancel_text: &str,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        let block = Block::default()
            .title(format!(" {title} "))
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            chunks[0],
        );
        let buttons = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);
        let confirm_style = if self.active_button == ModalButton::Confirm {
            Style::default()
                .bg(Color::Green)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };
        let cancel_style = if self.active_button == ModalButton::Cancel {
            Style::default()
                .bg(Color::Red)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red)
        };
        f.render_widget(
            Paragraph::new(confirm_text)
                .style(confirm_style)
                .alignment(Alignment::Center),
            buttons[0],
        );
        f.render_widget(
            Paragraph::new(cancel_text)
                .style(cancel_style)
                .alignment(Alignment::Center),
            buttons[1],
        );
    }

    fn render_info(&self, f: &mut Frame, area: Rect, title: &str, message: &str) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(3)])
            .split(area);
        let block = Block::default()
            .title(format!(" {title} "))
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black));
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            chunks[0],
        );
        let ok_style = if self.active_button == ModalButton::Ok {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Blue)
        };
        f.render_widget(
            Paragraph::new("[ Enter ] OK")
                .style(ok_style)
                .alignment(Alignment::Center),
            chunks[1],
        );
    }

    fn render_about(&self, f: &mut Frame, area: Rect) {
        //get ASCII art from a constant stored in theme.rs
        use super::theme::ASCII_ART;

        let version = env!("CARGO_PKG_VERSION");

        let about_text = format!(
            "{}\n\
            Version {}\n\
            \n\
            A terminal first remote monitoring tool\n\
            \n\
            Website: https://socktop.io\n\
            GitHub: https://github.com/jasonwitty/socktop\n\
            \n\
            License: MIT License\n\
            \n\
            Created by Jason Witty\n\
            jasonpwitty+socktop@proton.me",
            ASCII_ART, version
        );

        // Render the border block
        let block = Block::default()
            .title(" About socktop ")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black).fg(Color::DarkGray));
        f.render_widget(block, area);

        // Calculate inner area manually to avoid any parent styling
        let inner_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2), // Leave room for button at bottom
        };

        // Render content area with explicit black background
        f.render_widget(
            Paragraph::new(about_text)
                .style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            inner_area,
        );

        // Button area
        let button_area = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(2),
            width: area.width.saturating_sub(2),
            height: 1,
        };

        let ok_style = if self.active_button == ModalButton::Ok {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Blue).bg(Color::Black)
        };

        f.render_widget(
            Paragraph::new("[ Enter ] Close")
                .style(ok_style)
                .alignment(Alignment::Center),
            button_area,
        );
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let help_text = "\
GLOBAL
  q/Q/Esc ........ Quit  │  a/A ....... About  │  h/H ....... Help

PROCESS LIST
  / .............. Start/edit fuzzy search
  c/C ............ Clear search filter
  ↑/↓ ............ Select/navigate processes
  Enter .......... Open Process Details
  x/X ............ Clear selection
  Click header ... Sort by column (CPU/Mem)
  Click row ...... Select process

SEARCH MODE (after pressing /)
  Type ........... Enter search query (fuzzy match)
  ↑/↓ ............ Navigate results while typing
  Esc ............ Cancel search and clear filter
  Enter .......... Apply filter and select first result

CPU PER-CORE
  ←/→ ............ Scroll cores  │  PgUp/PgDn ... Page up/down
  Home/End ....... Jump to first/last core

PROCESS DETAILS MODAL
  x/X ............ Close modal (all parent modals)
  p/P ............ Navigate to parent process
  j/k ............ Scroll threads ↓/↑ (1 line)
  d/u ............ Scroll threads ↓/↑ (10 lines)
  [ / ] .......... Scroll journal ↑/↓
  Esc/Enter ...... Close modal

MODAL NAVIGATION
  Tab/→ .......... Next button  │  Shift+Tab/← ... Previous button
  Enter .......... Confirm/OK    │  Esc ............ Cancel/Close";
        // Render the border block
        let block = Block::default()
            .title(" Hotkey Help ")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black).fg(Color::DarkGray));
        f.render_widget(block, area);

        // Calculate inner area manually to avoid any parent styling
        let inner_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2), // Leave room for button at bottom
        };

        // Render content area with explicit black background
        f.render_widget(
            Paragraph::new(help_text)
                .style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .alignment(Alignment::Left)
                .wrap(Wrap { trim: false }),
            inner_area,
        );

        // Button area
        let button_area = Rect {
            x: area.x + 1,
            y: area.y + area.height.saturating_sub(2),
            width: area.width.saturating_sub(2),
            height: 1,
        };

        let ok_style = if self.active_button == ModalButton::Ok {
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Blue).bg(Color::Black)
        };

        f.render_widget(
            Paragraph::new("[ Enter ] Close")
                .style(ok_style)
                .alignment(Alignment::Center),
            button_area,
        );
    }

    fn centered_rect(&self, percent_x: u16, percent_y: u16, r: Rect) -> Rect {
        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(r);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(vert[1])[1]
    }
}
