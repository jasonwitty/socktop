//! Top header with hostname and CPU temperature indicator.

use crate::types::Metrics;
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders},
};

pub fn draw_header(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let title = if let Some(mm) = m {
        let temp = mm
            .cpu_temp_c
            .map(|t| {
                let icon = if t < 50.0 {
                    "😎"
                } else if t < 85.0 {
                    "⚠️"
                } else {
                    "🔥"
                };
                format!("CPU Temp: {:.1}°C {}", t, icon)
            })
            .unwrap_or_else(|| "CPU Temp: N/A".into());
        format!(
            "socktop — host: {} | {}  (press 'q' to quit)",
            mm.hostname, temp
        )
    } else {
        "socktop — connecting... (press 'q' to quit)".into()
    };
    f.render_widget(Block::default().title(title).borders(Borders::BOTTOM), area);
}
