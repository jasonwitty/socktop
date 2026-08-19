//! Disk cards with per-device gauge and title line.

use crate::types::Metrics;
use crate::ui::fit::truncate_middle_cols;
use crate::ui::util::{disk_icon, human};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Gauge},
};

pub fn draw_disks(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    f.render_widget(Block::default().borders(Borders::ALL).title("Disks"), area);
    let Some(mm) = m else {
        return;
    };

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height < 3 {
        return;
    }

    // Deduplication is performed once on the App side when fresh disk data
    // arrives (disks poll cadence is 5s, draw cadence is ~500ms, so doing it
    // here would rebuild a HashSet ~10x per refresh for no reason).
    let per_disk_h = 3u16;
    let max_cards = (inner.height / per_disk_h).min(mm.disks.len() as u16) as usize;

    let constraints: Vec<Constraint> = (0..max_cards)
        .map(|_| Constraint::Length(per_disk_h))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, slot) in rows.iter().enumerate() {
        let d = &mm.disks[i];
        let used = d.total.saturating_sub(d.available);
        let ratio = if d.total > 0 {
            used as f64 / d.total as f64
        } else {
            0.0
        };
        let pct = (ratio * 100.0).round() as u16;

        let color = if pct < 70 {
            ratatui::style::Color::Green
        } else if pct < 90 {
            ratatui::style::Color::Yellow
        } else {
            ratatui::style::Color::Red
        };

        // Add indentation for partitions
        let indent = if d.is_partition { "└─" } else { "" };

        // Add temperature if available
        let temp_str = d
            .temperature
            .map(|t| format!(" {}°C", t.round() as i32))
            .unwrap_or_default();

        let title = format!(
            "{}{}{}{}  {} / {}  ({}%)",
            indent,
            disk_icon(&d.name),
            truncate_middle_cols(&d.name, slot.width.saturating_sub(6) / 2),
            temp_str,
            human(used),
            human(d.total),
            pct
        );

        // Indent the entire card (block) for partitions to align with └─ prefix (4 chars)
        let card_indent = if d.is_partition { 4 } else { 0 };
        let card_rect = Rect {
            x: slot.x + card_indent,
            y: slot.y,
            width: slot.width.saturating_sub(card_indent),
            height: slot.height,
        };

        let card = Block::default().borders(Borders::ALL).title(title);
        f.render_widget(card, card_rect);

        let inner_card = Rect {
            x: card_rect.x + 1,
            y: card_rect.y + 1,
            width: card_rect.width.saturating_sub(2),
            height: card_rect.height.saturating_sub(2),
        };
        if inner_card.height == 0 {
            continue;
        }

        let gauge_rect = Rect {
            x: inner_card.x,
            y: inner_card.y + inner_card.height / 2,
            width: inner_card.width,
            height: 1,
        };

        let g = Gauge::default()
            .percent(pct)
            .gauge_style(Style::default().fg(color));

        f.render_widget(g, gauge_rect);
    }
}
