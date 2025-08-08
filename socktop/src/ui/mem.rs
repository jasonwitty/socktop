//! Memory gauge.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Gauge},
};
use crate::types::Metrics;
use crate::ui::util::human;

pub fn draw_mem(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.mem_total > 0 { (mm.mem_used as f64 / mm.mem_total as f64 * 100.0) as u16 } else { 0 };
        (mm.mem_used, mm.mem_total, pct)
    } else { (0, 0, 0) };

    let g = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory"))
        .gauge_style(Style::default().fg(Color::Magenta))
        .percent(pct)
        .label(format!("{} / {}", human(used), human(total)));
    f.render_widget(g, area);
}