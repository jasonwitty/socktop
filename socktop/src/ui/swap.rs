//! Swap gauge.

use crate::types::Metrics;
use crate::ui::util::{draw_labeled_gauge, human};
use ratatui::{layout::Rect, style::Color};

pub fn draw_swap(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let (used, total, pct) = if let Some(mm) = m {
        let pct = if mm.swap_total > 0 {
            (mm.swap_used as f64 / mm.swap_total as f64 * 100.0) as u16
        } else {
            0
        };
        (mm.swap_used, mm.swap_total, pct)
    } else {
        (0, 0, 0)
    };

    draw_labeled_gauge(
        f,
        area,
        "Swap",
        Color::Yellow,
        pct,
        format!("{} / {}", human(used), human(total)),
    );
}
