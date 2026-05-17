//! Connection error modal rendering

use std::time::Instant;

use super::modal_format::format_duration;
use super::theme::{
    BTN_EXIT_BG_ACTIVE, BTN_EXIT_FG_ACTIVE, BTN_EXIT_FG_INACTIVE, BTN_EXIT_TEXT,
    BTN_RETRY_BG_ACTIVE, BTN_RETRY_FG_ACTIVE, BTN_RETRY_FG_INACTIVE, BTN_RETRY_TEXT, ICON_CLUSTER,
    ICON_COUNTDOWN_LABEL, ICON_MESSAGE, ICON_OFFLINE_LABEL, ICON_RETRY_LABEL, ICON_WARNING_TITLE,
    LARGE_ERROR_ICON, MODAL_AGENT_FG, MODAL_BG, MODAL_BORDER_FG, MODAL_COUNTDOWN_LABEL_FG,
    MODAL_FG, MODAL_HINT_FG, MODAL_ICON_PINK, MODAL_OFFLINE_LABEL_FG, MODAL_RETRY_LABEL_FG,
    MODAL_TITLE_FG,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::modal::{ModalButton, ModalManager};

impl ModalManager {
    pub(super) fn render_connection_error(
        &self,
        f: &mut Frame,
        area: Rect,
        message: &str,
        disconnected_at: Instant,
        retry_count: u32,
        auto_retry_countdown: Option<u64>,
    ) {
        let duration_text = format_duration(disconnected_at.elapsed());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(4),
                Constraint::Length(4),
            ])
            .split(area);
        let block = Block::default()
            .title(Line::from(ICON_WARNING_TITLE).style(
                Style::default()
                    .fg(MODAL_TITLE_FG)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MODAL_BORDER_FG))
            .style(Style::default().bg(MODAL_BG).fg(MODAL_FG));
        f.render_widget(block, area);

        let content_area = chunks[1];
        let max_w = content_area.width.saturating_sub(15) as usize;
        let clean_message = if message.to_lowercase().contains("hostname verification")
            || message.contains("socktop_connector")
        {
            "Connection failed - hostname verification disabled".to_string()
        } else if message.contains("Failed to fetch metrics:") {
            if let Some(p) = message.find(':') {
                let ess = message[p + 1..].trim();
                if ess.len() > max_w {
                    format!("{}...", &ess[..max_w.saturating_sub(3)])
                } else {
                    ess.to_string()
                }
            } else {
                "Connection error".to_string()
            }
        } else if message.starts_with("Retry failed:") {
            if let Some(p) = message.find(':') {
                let ess = message[p + 1..].trim();
                if ess.len() > max_w {
                    format!("{}...", &ess[..max_w.saturating_sub(3)])
                } else {
                    ess.to_string()
                }
            } else {
                "Retry failed".to_string()
            }
        } else if message.len() > max_w {
            format!("{}...", &message[..max_w.saturating_sub(3)])
        } else {
            message.to_string()
        };
        let truncate = |s: &str| {
            if s.len() > max_w {
                format!("{}...", &s[..max_w.saturating_sub(3)])
            } else {
                s.to_string()
            }
        };
        let agent_text = truncate("📡 Cannot connect to socktop agent");
        let message_text = truncate(&clean_message);
        let duration_display = truncate(&duration_text);
        let retry_display = truncate(&retry_count.to_string());
        let countdown_text = auto_retry_countdown.map(|c| {
            if c == 0 {
                "Auto retry now...".to_string()
            } else {
                format!("{c}s")
            }
        });

        // Determine if we have enough space (height + width) to show large centered icon
        let icon_max_width = LARGE_ERROR_ICON
            .iter()
            .map(|l| l.trim().chars().count())
            .max()
            .unwrap_or(0) as u16;
        let large_allowed = content_area.height >= (LARGE_ERROR_ICON.len() as u16 + 8)
            && content_area.width >= icon_max_width + 6; // small margin for borders/padding
        let mut icon_lines: Vec<Line> = Vec::new();
        if large_allowed {
            for &raw in LARGE_ERROR_ICON.iter() {
                let trimmed = raw.trim();
                icon_lines.push(Line::from(
                    trimmed
                        .chars()
                        .map(|ch| {
                            if ch == '!' {
                                Span::styled(
                                    ch.to_string(),
                                    Style::default()
                                        .fg(Color::White)
                                        .add_modifier(Modifier::BOLD),
                                )
                            } else if ch == '/' || ch == '\\' || ch == '_' {
                                // keep outline in pink
                                Span::styled(
                                    ch.to_string(),
                                    Style::default()
                                        .fg(MODAL_ICON_PINK)
                                        .add_modifier(Modifier::BOLD),
                                )
                            } else if ch == ' ' {
                                Span::raw(" ")
                            } else {
                                Span::styled(ch.to_string(), Style::default().fg(MODAL_ICON_PINK))
                            }
                        })
                        .collect::<Vec<_>>(),
                ));
            }
            icon_lines.push(Line::from("")); // blank spacer line below icon
        }

        let mut info_lines: Vec<Line> = Vec::new();
        if !large_allowed {
            info_lines.push(Line::from(vec![Span::styled(
                ICON_CLUSTER,
                Style::default().fg(MODAL_ICON_PINK),
            )]));
            info_lines.push(Line::from(""));
        }
        info_lines.push(Line::from(vec![Span::styled(
            &agent_text,
            Style::default().fg(MODAL_AGENT_FG),
        )]));
        info_lines.push(Line::from(""));
        info_lines.push(Line::from(vec![
            Span::styled(ICON_MESSAGE, Style::default().fg(MODAL_HINT_FG)),
            Span::styled(&message_text, Style::default().fg(MODAL_AGENT_FG)),
        ]));
        info_lines.push(Line::from(""));
        info_lines.push(Line::from(vec![
            Span::styled(
                ICON_OFFLINE_LABEL,
                Style::default().fg(MODAL_OFFLINE_LABEL_FG),
            ),
            Span::styled(
                &duration_display,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        info_lines.push(Line::from(vec![
            Span::styled(ICON_RETRY_LABEL, Style::default().fg(MODAL_RETRY_LABEL_FG)),
            Span::styled(
                &retry_display,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if let Some(cd) = &countdown_text {
            info_lines.push(Line::from(vec![
                Span::styled(
                    ICON_COUNTDOWN_LABEL,
                    Style::default().fg(MODAL_COUNTDOWN_LABEL_FG),
                ),
                Span::styled(
                    cd,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        let constrained = Rect {
            x: content_area.x + 2,
            y: content_area.y,
            width: content_area.width.saturating_sub(4),
            height: content_area.height,
        };
        if large_allowed {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(icon_lines.len() as u16),
                    Constraint::Min(0),
                ])
                .split(constrained);
            // Center the icon block; each line already trimmed so per-line centering keeps shape
            f.render_widget(
                Paragraph::new(Text::from(icon_lines))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false }),
                split[0],
            );
            f.render_widget(
                Paragraph::new(Text::from(info_lines))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                split[1],
            );
        } else {
            f.render_widget(
                Paragraph::new(Text::from(info_lines))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                constrained,
            );
        }

        let button_area = Rect {
            x: chunks[2].x,
            y: chunks[2].y,
            width: chunks[2].width,
            height: chunks[2].height.saturating_sub(1),
        };
        self.render_connection_error_buttons(f, button_area);
    }

    fn render_connection_error_buttons(&self, f: &mut Frame, area: Rect) {
        let button_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(15),
                Constraint::Percentage(10),
                Constraint::Percentage(15),
                Constraint::Percentage(30),
            ])
            .split(area);
        let retry_style = if self.active_button == ModalButton::Retry {
            Style::default()
                .bg(BTN_RETRY_BG_ACTIVE)
                .fg(BTN_RETRY_FG_ACTIVE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(BTN_RETRY_FG_INACTIVE)
                .add_modifier(Modifier::DIM)
        };
        let exit_style = if self.active_button == ModalButton::Exit {
            Style::default()
                .bg(BTN_EXIT_BG_ACTIVE)
                .fg(BTN_EXIT_FG_ACTIVE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(BTN_EXIT_FG_INACTIVE)
                .add_modifier(Modifier::DIM)
        };
        f.render_widget(
            Paragraph::new(Text::from(Line::from(vec![Span::styled(
                BTN_RETRY_TEXT,
                retry_style,
            )])))
            .alignment(Alignment::Center),
            button_chunks[1],
        );
        f.render_widget(
            Paragraph::new(Text::from(Line::from(vec![Span::styled(
                BTN_EXIT_TEXT,
                exit_style,
            )])))
            .alignment(Alignment::Center),
            button_chunks[3],
        );
    }
}
