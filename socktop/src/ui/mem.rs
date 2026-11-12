//! Memory gauge.

use crate::types::Metrics;
use crate::ui::util::{draw_labeled_gauge, human};
use ratatui::{layout::Rect, style::Color};

pub fn draw_mem(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.mem_total > 0 {
            (mm.mem_used as f64 / mm.mem_total as f64 * 100.0) as u16
        } else {
            0
        };
        (mm.mem_used, mm.mem_total, pct)
    } else {
        (0, 0, 0)
    };

    draw_labeled_gauge(
        f,
        area,
        "Memory",
        Color::Magenta,
        pct,
        format!("{} / {}", human(used), human(total)),
    );
}
