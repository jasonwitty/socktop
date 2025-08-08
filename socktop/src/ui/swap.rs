//! Swap gauge.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Gauge},
};
use crate::types::Metrics;
use crate::ui::util::human;

pub fn draw_swap(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.swap_total > 0 { (mm.swap_used as f64 / mm.swap_total as f64 * 100.0) as u16 } else { 0 };
        (mm.swap_used, mm.swap_total, pct)
    } else { (0, 0, 0) };

    let g = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Swap"))
        .gauge_style(Style::default().fg(Color::Yellow))
        .percent(pct)
        .label(format!("{} / {}", human(used), human(total)));
    f.render_widget(g, area);
}