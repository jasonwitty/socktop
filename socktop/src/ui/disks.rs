//! Disk cards with per-device gauge and title line.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Gauge},
};
use crate::types::Metrics;
use crate::ui::util::{human, truncate_middle, disk_icon};

pub fn draw_disks(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    f.render_widget(Block::default().borders(Borders::ALL).title("Disks"), area);
    let Some(mm) = m else { return; };

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height < 3 { return; }

    let per_disk_h = 3u16;
    let max_cards = (inner.height / per_disk_h).min(mm.disks.len() as u16) as usize;

    let constraints: Vec<Constraint> = (0..max_cards).map(|_| Constraint::Length(per_disk_h)).collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, slot) in rows.iter().enumerate() {
        let d = &mm.disks[i];
        let used = d.total.saturating_sub(d.available);
        let ratio = if d.total > 0 { used as f64 / d.total as f64 } else { 0.0 };
        let pct = (ratio * 100.0).round() as u16;

        let color = if pct < 70 { ratatui::style::Color::Green } else if pct < 90 { ratatui::style::Color::Yellow } else { ratatui::style::Color::Red };

        let title = format!(
            "{} {}   {} / {}  ({}%)",
            disk_icon(&d.name),
            truncate_middle(&d.name, (slot.width.saturating_sub(6)) as usize / 2),
            human(used),
            human(d.total),
            pct
        );

        let card = Block::default().borders(Borders::ALL).title(title);
        f.render_widget(card, *slot);

        let inner_card = Rect {
            x: slot.x + 1,
            y: slot.y + 1,
            width: slot.width.saturating_sub(2),
            height: slot.height.saturating_sub(2),
        };
        if inner_card.height == 0 { continue; }

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