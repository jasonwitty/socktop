//! Modal window system for socktop TUI application

use super::fit;
use super::theme::{
    BTN_EXIT_BG_ACTIVE, BTN_RETRY_BG_ACTIVE, MODAL_BG, MODAL_BORDER_FG, MODAL_DIM_BG, MODAL_FG,
    MODAL_TITLE_FG,
};
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
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
    pub help_scroll_offset: usize,
}

/// Key hints shown under the confirmation buttons. Also sets the minimum
/// width of that dialog — sizing from the question alone clipped this line.
const CONFIRM_HINT: &str = "Tab ← → choose  ·  Enter run  ·  Esc cancel";

impl ModalManager {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            active_button: ModalButton::Retry,
            thread_scroll_offset: 0,
            journal_scroll_offset: 0,
            thread_scroll_max: 0,
            journal_scroll_max: 0,
            help_scroll_offset: 0,
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
            Some(ModalType::Help) => {
                // Reset scroll state for help modal
                self.help_scroll_offset = 0;
                ModalButton::Ok
            }
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
    /// Close the details view for `pid` WHEREVER it sits in the stack.
    /// Returns whether anything was closed.
    ///
    /// Not just the top: killing from inside the details view stacks the
    /// "Signal sent" Info modal on top of it, and a SIGKILL victim is usually
    /// confirmed dead on the very next tick — while that Info is still up. A
    /// top-only check missed the close, and since a gone PID is processed
    /// once, the details view stayed open (frozen on the dead process's last
    /// sample) with nothing left to ever close it.
    ///
    /// Per-PID matching keeps the parent-navigation property: only the dead
    /// process's view goes; parent views underneath are other processes that
    /// may still be alive and close themselves the same way.
    pub fn close_process_details(&mut self, pid: u32) -> bool {
        let was_top =
            matches!(self.stack.last(), Some(ModalType::ProcessDetails { pid: p }) if *p == pid);
        let before = self.stack.len();
        self.stack
            .retain(|m| !matches!(m, ModalType::ProcessDetails { pid: p } if *p == pid));
        if self.stack.len() == before {
            return false;
        }
        // Mirror pop_modal's focus bookkeeping when the top changed.
        if was_top && let Some(next) = self.stack.last() {
            self.active_button = match next {
                ModalType::ConnectionError { .. } => ModalButton::Retry,
                ModalType::ProcessDetails { .. } => ModalButton::Ok,
                ModalType::About => ModalButton::Ok,
                ModalType::Help => ModalButton::Ok,
                ModalType::Confirmation { .. } => ModalButton::Confirm,
                ModalType::Info { .. } => ModalButton::Ok,
            };
        }
        true
    }

    /// PID of the uppermost ProcessDetails view, looking through any
    /// Info/Confirmation stacked above it. What the user will land on when
    /// transient modals are dismissed.
    pub fn topmost_process_details(&self) -> Option<u32> {
        self.stack.iter().rev().find_map(|m| match m {
            ModalType::ProcessDetails { pid } => Some(*pid),
            _ => None,
        })
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
            // Kill the process being viewed. `t` rather than `k` because `k`
            // scrolls the thread table in this modal — and using the same key
            // here as on the processes pane means one thing to remember.
            KeyCode::Char('t') | KeyCode::Char('T') => {
                if let Some(ModalType::ProcessDetails { pid }) = self.stack.last() {
                    ModalAction::KillSelected(*pid)
                } else {
                    ModalAction::None
                }
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
            KeyCode::Up => {
                if matches!(self.stack.last(), Some(ModalType::Help)) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_sub(1);
                    ModalAction::Handled
                } else {
                    ModalAction::None
                }
            }
            KeyCode::Down => {
                if matches!(self.stack.last(), Some(ModalType::Help)) {
                    self.help_scroll_offset = self.help_scroll_offset.saturating_add(1);
                    ModalAction::Handled
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
            (Some(ModalType::Confirmation { .. }), ModalButton::ConfirmForce) => {
                ModalAction::ConfirmForce
            }
            (Some(ModalType::Confirmation { .. }), ModalButton::Cancel) => {
                // Pop here so Enter-on-Cancel behaves like Esc (which pops in
                // handle_key); the app's Cancel handler can then assume the
                // modal is already gone.
                self.pop_modal();
                ModalAction::Cancel
            }
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
            // Confirmation cycles through three: the safe affirmative, the
            // escalated one, then cancel.
            (Some(ModalType::Confirmation { .. }), ModalButton::Confirm) => {
                ModalButton::ConfirmForce
            }
            (Some(ModalType::Confirmation { .. }), ModalButton::ConfirmForce) => {
                ModalButton::Cancel
            }
            (Some(ModalType::Confirmation { .. }), ModalButton::Cancel) => ModalButton::Confirm,
            _ => self.active_button.clone(),
        };
    }
    fn prev_button(&mut self) {
        // Confirmation has three buttons, so stepping back is not the same as
        // stepping forward; everything else is a two-way toggle.
        if let Some(ModalType::Confirmation { .. }) = self.stack.last() {
            self.active_button = match self.active_button {
                ModalButton::Confirm => ModalButton::Cancel,
                ModalButton::ConfirmForce => ModalButton::Confirm,
                _ => ModalButton::ConfirmForce,
            };
            return;
        }
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

    /// Wrap `text` to at most `width` columns on word boundaries, so a dialog
    /// can be sized from its content instead of guessing.
    fn wrap_cols(text: &str, width: u16) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for word in text.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if fit::cols(&candidate) <= width || current.is_empty() {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    /// A centered box just big enough for `message` plus `footer_rows` of
    /// buttons/hints. Never exceeds the screen, and never gets so narrow that
    /// the title is clipped.
    ///
    /// `min_content_w` is the width the footer needs. Sizing from the message
    /// alone clipped the key-hint line, which is longer than most questions.
    fn dialog_rect(area: Rect, message: &str, footer_rows: u16, min_content_w: u16) -> Rect {
        // 2 border columns + 2 columns of breathing room on each side.
        const CHROME_W: u16 = 6;
        const MAX_TEXT_W: u16 = 64;
        const MIN_TEXT_W: u16 = 24;

        let avail_text = area.width.saturating_sub(CHROME_W).max(1);
        let text_w = fit::cols(message)
            .min(MAX_TEXT_W)
            .min(avail_text)
            .max(MIN_TEXT_W.min(avail_text));
        let lines = Self::wrap_cols(message, text_w);
        let widest = lines
            .iter()
            .map(|l| fit::cols(l))
            .max()
            .unwrap_or(text_w)
            .max(min_content_w.min(avail_text));

        let width = (widest + CHROME_W).min(area.width);
        // borders + blank + message + blank + footer
        let height = (lines.len() as u16 + footer_rows + 4).min(area.height);
        Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        }
    }

    /// Shared chrome for the small dialogs: themed border, centered message
    /// with real padding, and the footer row(s) returned for the caller to
    /// fill with buttons.
    ///
    /// The old versions laid their content out over `area` rather than the
    /// block's inner rect, which put the first line of text on top of the
    /// border and pushed the buttons against the frame.
    fn render_dialog_frame(
        f: &mut Frame,
        area: Rect,
        title: &str,
        message: &str,
        footer_rows: u16,
    ) -> Rect {
        let block = Block::default()
            .title(
                Line::from(format!(" {title} ")).style(
                    Style::default()
                        .fg(MODAL_TITLE_FG)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MODAL_BORDER_FG))
            .style(Style::default().bg(MODAL_BG));
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Pad one column each side so text never touches the border.
        let padded = Rect {
            x: inner.x + 1,
            y: inner.y,
            width: inner.width.saturating_sub(2),
            height: inner.height,
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),           // breathing room under the title
                Constraint::Min(1),              // message
                Constraint::Length(1),           // gap above the footer
                Constraint::Length(footer_rows), // buttons / hints
            ])
            .split(padded);

        f.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(MODAL_FG))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            rows[1],
        );
        rows[3]
    }

    /// One button, sized to its label and centered in `area`.
    fn render_button(f: &mut Frame, area: Rect, label: &str, active: bool, accent: Color) {
        let style = if active {
            Style::default()
                .bg(accent)
                .fg(MODAL_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(accent)
        };
        let text = format!(" {label} ");
        let w = fit::cols(&text).min(area.width);
        let btn = Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y,
            width: w,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(text)
                .style(style)
                .alignment(Alignment::Center),
            btn,
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
            // Confirmation and Info are one-question dialogs. A fixed 70%x50%
            // box left a short question floating in a mostly-empty pane, so
            // these size themselves to their content instead.
            ModalType::Confirmation { message, .. } => {
                Self::dialog_rect(area, message, 3, fit::cols(CONFIRM_HINT))
            }
            ModalType::Info { message, .. } => Self::dialog_rect(area, message, 1, 16),
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
        // Three buttons + a key hint line.
        let footer = Self::render_dialog_frame(f, area, title, message, 3);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // buttons
                Constraint::Length(1), // spacer
                Constraint::Length(1), // key hints
            ])
            .split(footer);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(rows[0]);

        Self::render_button(
            f,
            cols[0],
            confirm_text,
            self.active_button == ModalButton::Confirm,
            BTN_RETRY_BG_ACTIVE,
        );
        Self::render_button(
            f,
            cols[1],
            "Force kill",
            self.active_button == ModalButton::ConfirmForce,
            MODAL_TITLE_FG,
        );
        Self::render_button(
            f,
            cols[2],
            cancel_text,
            self.active_button == ModalButton::Cancel,
            BTN_EXIT_BG_ACTIVE,
        );

        f.render_widget(
            Paragraph::new(CONFIRM_HINT)
                .style(Style::default().fg(MODAL_FG).add_modifier(Modifier::DIM))
                .alignment(Alignment::Center),
            rows[2],
        );
    }

    fn render_info(&self, f: &mut Frame, area: Rect, title: &str, message: &str) {
        let footer = Self::render_dialog_frame(f, area, title, message, 1);
        Self::render_button(
            f,
            footer,
            "Enter — OK",
            self.active_button == ModalButton::Ok,
            BTN_RETRY_BG_ACTIVE,
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
        let help_lines = vec![
            "GLOBAL",
            "  q/Q/Esc ........ Quit  │  a/A ....... About  │  h/H ....... Help",
            "",
            "PROCESS LIST",
            "  / .............. Start/edit fuzzy search",
            "  c/C ............ Clear search filter",
            "  ↑/↓ ............ Select/navigate processes",
            "  Enter .......... Open Process Details",
            "  x/X ............ Clear selection",
            "  t .............. Signal selected process — local agent only",
            "                   (also works inside Process Details; the prompt",
            "                    offers Terminate/SIGTERM or Force kill/SIGKILL)",
            "  Click header ... Sort by column (CPU/Mem)",
            "  Click row ...... Select process",
            "",
            "SEARCH MODE (after pressing /)",
            "  Type ........... Enter search query (fuzzy match)",
            "  ↑/↓ ............ Navigate results while typing",
            "  Esc ............ Cancel search and clear filter",
            "  Enter .......... Apply filter and select first result",
            "",
            "CPU PER-CORE",
            "  ←/→ ............ Scroll cores  │  PgUp/PgDn ... Page up/down",
            "  Home/End ....... Jump to first/last core",
            "",
            "PROCESS DETAILS MODAL",
            "  x/X ............ Close modal (all parent modals)",
            "  p/P ............ Navigate to parent process",
            "  j/k ............ Scroll threads ↓/↑ (1 line)",
            "  d/u ............ Scroll threads ↓/↑ (10 lines)",
            "  [ / ] .......... Scroll journal ↑/↓",
            "  Esc/Enter ...... Close modal",
            "",
            "MODAL NAVIGATION",
            "  Tab/→ .......... Next button  │  Shift+Tab/← ... Previous button",
            "  Enter .......... Confirm/OK    │  Esc ............ Cancel/Close",
        ];

        // Render the border block
        let block = Block::default()
            .title(" Hotkey Help (use ↑/↓ to scroll) ")
            .borders(Borders::ALL)
            .style(Style::default().bg(Color::Black).fg(Color::DarkGray));
        f.render_widget(block, area);

        // Split into content area and button area
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width.saturating_sub(2),
                height: area.height.saturating_sub(2),
            });

        let content_area = chunks[0];
        let button_area = chunks[1];

        // Calculate visible window
        let visible_height = content_area.height as usize;
        let total_lines = help_lines.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let scroll_offset = self.help_scroll_offset.min(max_scroll);

        // Get visible lines
        let visible_lines: Vec<Line> = help_lines
            .iter()
            .skip(scroll_offset)
            .take(visible_height)
            .map(|s| Line::from(*s))
            .collect();

        // Render scrollable content
        f.render_widget(
            Paragraph::new(visible_lines)
                .style(Style::default().fg(Color::Cyan).bg(Color::Black))
                .alignment(Alignment::Left),
            content_area,
        );

        // Render scrollbar if needed
        if total_lines > visible_height {
            use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

            let scrollbar_area = Rect {
                x: area.x + area.width.saturating_sub(2),
                y: area.y + 1,
                width: 1,
                height: area.height.saturating_sub(2),
            };

            let mut scrollbar_state = ScrollbarState::new(max_scroll).position(scroll_offset);

            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"))
                .style(Style::default().fg(Color::DarkGray));

            f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }

        // Button area
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

#[cfg(test)]
mod confirm_tests {
    use super::*;

    fn confirm_modal() -> ModalManager {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::Confirmation {
            title: "Confirm signal".into(),
            message: "Send a signal to bash (PID 42)?".into(),
            confirm_text: "Terminate".into(),
            cancel_text: "Cancel".into(),
        });
        m
    }

    /// The safe option is focused first, so a reflexive Enter terminates rather
    /// than force-kills.
    #[test]
    fn opens_on_the_safe_option() {
        let mut m = confirm_modal();
        assert_eq!(m.active_button, ModalButton::Confirm);
        assert_eq!(m.handle_key(KeyCode::Enter), ModalAction::Confirm);
    }

    #[test]
    fn tab_cycles_all_three_buttons_forward() {
        let mut m = confirm_modal();
        m.handle_key(KeyCode::Tab);
        assert_eq!(m.active_button, ModalButton::ConfirmForce);
        m.handle_key(KeyCode::Tab);
        assert_eq!(m.active_button, ModalButton::Cancel);
        m.handle_key(KeyCode::Tab);
        assert_eq!(m.active_button, ModalButton::Confirm);
    }

    /// With three buttons, back is not the same as forward — the old
    /// prev_button just called next_button, which only worked for two.
    #[test]
    fn shift_tab_cycles_backward() {
        let mut m = confirm_modal();
        m.handle_key(KeyCode::BackTab);
        assert_eq!(m.active_button, ModalButton::Cancel);
        m.handle_key(KeyCode::BackTab);
        assert_eq!(m.active_button, ModalButton::ConfirmForce);
        m.handle_key(KeyCode::BackTab);
        assert_eq!(m.active_button, ModalButton::Confirm);
    }

    #[test]
    fn force_kill_reports_its_own_action() {
        let mut m = confirm_modal();
        m.handle_key(KeyCode::Tab);
        assert_eq!(m.handle_key(KeyCode::Enter), ModalAction::ConfirmForce);
    }

    #[test]
    fn escape_cancels_and_closes() {
        let mut m = confirm_modal();
        assert_eq!(m.handle_key(KeyCode::Esc), ModalAction::Cancel);
        assert!(!m.is_active());
    }

    /// Enter on Cancel must behave like Esc, including closing the modal.
    #[test]
    fn enter_on_cancel_closes_too() {
        let mut m = confirm_modal();
        m.handle_key(KeyCode::Tab);
        m.handle_key(KeyCode::Tab);
        assert_eq!(m.handle_key(KeyCode::Enter), ModalAction::Cancel);
        assert!(!m.is_active());
    }

    /// `t` inside process details asks the app to raise the kill prompt for the
    /// process being viewed — not for whatever is selected in the list behind it.
    #[test]
    fn t_in_process_details_targets_that_pid() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 4242 });
        assert_eq!(
            m.handle_key(KeyCode::Char('t')),
            ModalAction::KillSelected(4242)
        );
    }

    /// `k` still scrolls the thread table, which is why `t` is the kill key.
    #[test]
    fn k_in_process_details_still_scrolls() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 1 });
        m.thread_scroll_max = 5;
        m.handle_key(KeyCode::Char('j'));
        assert_eq!(m.thread_scroll_offset, 1);
        assert_eq!(m.handle_key(KeyCode::Char('k')), ModalAction::Handled);
        assert_eq!(m.thread_scroll_offset, 0);
    }

    #[test]
    fn t_elsewhere_is_not_a_kill() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::Help);
        assert_eq!(m.handle_key(KeyCode::Char('t')), ModalAction::None);
    }

    /// A one-line question must not be handed a half-screen box.
    #[test]
    fn dialog_is_sized_to_its_content() {
        let screen = Rect::new(0, 0, 120, 40);
        let r = ModalManager::dialog_rect(
            screen,
            "Send a signal to bash (PID 42)?",
            3,
            fit::cols(CONFIRM_HINT),
        );
        assert!(r.width < screen.width, "dialog took the full width");
        assert!(r.height <= 12, "dialog was {} rows tall", r.height);
        assert!(r.height >= 7, "dialog too short to hold its own footer");
        // Centered to within the rounding of integer division.
        let center_delta = (r.x + r.width / 2) as i32 - (screen.width / 2) as i32;
        assert!(center_delta.abs() <= 1, "off-center by {center_delta}");
    }

    #[test]
    fn dialog_never_exceeds_a_small_screen() {
        let screen = Rect::new(0, 0, 20, 8);
        let long = "a".repeat(400);
        let r = ModalManager::dialog_rect(screen, &long, 3, fit::cols(CONFIRM_HINT));
        assert!(r.width <= screen.width && r.height <= screen.height);
    }
}

#[cfg(test)]
mod button_style_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The focused button must be the highlighted one — the whole point of the
    /// three-button layout is that you can see which action Enter will run.
    #[test]
    fn focus_moves_the_highlight() {
        let msg = "Send a signal to bash (PID 42)?";
        let mut m = ModalManager::new();
        m.push_modal(ModalType::Confirmation {
            title: "Confirm signal".into(),
            message: msg.into(),
            confirm_text: "Terminate".into(),
            cancel_text: "Cancel".into(),
        });

        // Background colors present on the button row, per focused button.
        let bgs = |m: &ModalManager| -> Vec<Color> {
            let screen = Rect::new(0, 0, 100, 30);
            let area = ModalManager::dialog_rect(screen, msg, 3, fit::cols(CONFIRM_HINT));
            let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
            t.draw(|f| {
                m.render_confirmation(f, area, "Confirm signal", msg, "Terminate", "Cancel")
            })
            .unwrap();
            let buf = t.backend().buffer();
            // Buttons sit on the first footer row: title, gap, message, gap.
            let row = area.y + 4;
            (area.x..area.x + area.width)
                .map(|x| buf[(x, row)].bg)
                .collect()
        };

        let terminate_focused = bgs(&m);
        assert!(
            terminate_focused.contains(&BTN_RETRY_BG_ACTIVE),
            "Terminate should be highlighted when focused"
        );
        assert!(
            !terminate_focused.contains(&BTN_EXIT_BG_ACTIVE),
            "Cancel must not be highlighted while Terminate has focus"
        );

        m.handle_key(KeyCode::Tab);
        m.handle_key(KeyCode::Tab);
        let cancel_focused = bgs(&m);
        assert!(
            cancel_focused.contains(&BTN_EXIT_BG_ACTIVE),
            "Cancel should be highlighted after two Tabs"
        );
        assert!(
            !cancel_focused.contains(&BTN_RETRY_BG_ACTIVE),
            "Terminate must not stay highlighted"
        );
    }
}

#[cfg(test)]
mod close_details_tests {
    use super::*;

    #[test]
    fn closes_the_view_for_that_pid() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 4242 });
        assert!(m.close_process_details(4242));
        assert!(!m.is_active());
    }

    #[test]
    fn leaves_a_different_pid_alone() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 4242 });
        assert!(!m.close_process_details(1));
        assert!(m.is_active());
    }

    /// Walking up to a parent stacks details views. Only the dead process's
    /// view goes — whichever position it holds — and the survivor stays put.
    #[test]
    fn closes_only_the_dead_pids_view_in_a_parent_chain() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 100 }); // parent
        m.push_modal(ModalType::ProcessDetails { pid: 200 }); // child, on top

        // Parent dies while the child is viewed: its view is removed from
        // UNDER the top, so closing the child later lands on the process list
        // instead of a frozen corpse view.
        assert!(m.close_process_details(100));
        assert!(matches!(
            m.current_modal(),
            Some(ModalType::ProcessDetails { pid: 200 })
        ));
        assert!(m.close_process_details(200));
        assert!(!m.is_active());
    }

    /// The F1 regression: killing from inside the details view stacks the
    /// "Signal sent" Info on top, and the death is usually confirmed while
    /// that Info is still up. The details view must close anyway — a top-only
    /// check left it open forever, frozen on the dead process.
    #[test]
    fn closes_details_beneath_a_stacked_info_modal() {
        let mut m = ModalManager::new();
        m.push_modal(ModalType::ProcessDetails { pid: 7 });
        m.push_modal(ModalType::Info {
            title: "Signal sent".into(),
            message: "Sent SIGKILL".into(),
        });

        assert!(m.close_process_details(7));
        // The Info survives on top; dismissing it lands on the process list.
        assert!(matches!(m.current_modal(), Some(ModalType::Info { .. })));
        m.pop_modal();
        assert!(!m.is_active());
    }
}
