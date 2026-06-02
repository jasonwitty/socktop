//! Top header with hostname and CPU temperature indicator.

use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Build the header's left-side title from session state. Callers cache the
/// returned String and only rebuild it when one of the inputs changes.
pub fn build_header_title(hostname: Option<&str>, is_tls: bool, has_token: bool) -> String {
    let base = match hostname {
        Some(h) => format!("socktop — host: {h}"),
        None => "socktop — connecting...".into(),
    };
    let tls_txt = if is_tls { "🔒 TLS" } else { "🔒✗ TLS" };
    let mut parts = vec![base, tls_txt.into()];
    if has_token {
        parts.push("🔑 token".into());
    }
    parts.push("(a: about, h: help, q: quit)".into());
    parts.join(" | ")
}

/// Build the right-side polling interval text. Callers cache this string.
pub fn build_header_intervals(metrics_ms: u128, procs_ms: u128) -> String {
    format!("⏱ {metrics_ms}ms metrics | {procs_ms}ms procs")
}

pub fn draw_header(f: &mut ratatui::Frame<'_>, area: Rect, title: &str, intervals: &str) {
    f.render_widget(Block::default().title(title).borders(Borders::BOTTOM), area);

    let intervals_width = intervals.len() as u16;
    if area.width > intervals_width + 2 {
        let right_area = Rect {
            x: area.x + area.width.saturating_sub(intervals_width + 1),
            y: area.y,
            width: intervals_width,
            height: 1,
        };
        let intervals_line = Line::from(Span::raw(intervals));
        f.render_widget(Paragraph::new(intervals_line), right_area);
    }
}
