//! CPU average sparkline + per-core mini bars.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
};
use ratatui::style::Modifier;

use crate::history::PerCoreHistory;
use crate::types::Metrics;

pub fn draw_cpu_avg_graph(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    hist: &std::collections::VecDeque<u64>,
    m: Option<&Metrics>,
) {
    let title = if let Some(mm) = m { format!("CPU avg (now: {:>5.1}%)", mm.cpu_total) } else { "CPU avg".into() };
    let max_points = area.width.saturating_sub(2) as usize;
    let start = hist.len().saturating_sub(max_points);
    let data: Vec<u64> = hist.iter().skip(start).cloned().collect();
    let spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(&data)
        .max(100)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(spark, area);
}

pub fn draw_per_core_bars(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    m: Option<&Metrics>,
    per_core_hist: &PerCoreHistory,
) {
    f.render_widget(Block::default().borders(Borders::ALL).title("Per-core"), area);
    let Some(mm) = m else { return; };

    let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(2), height: area.height.saturating_sub(2) };
    if inner.height == 0 { return; }

    let rows = inner.height as usize;
    let show_n = rows.min(mm.cpu_per_core.len());
    let constraints: Vec<Constraint> = (0..show_n).map(|_| Constraint::Length(1)).collect();
    let vchunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(inner);

    for i in 0..show_n {
        let rect = vchunks[i];
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(6), Constraint::Length(12)])
            .split(rect);

        let curr = mm.cpu_per_core[i].clamp(0.0, 100.0);
        let older = per_core_hist.deques.get(i)
            .and_then(|d| d.iter().rev().nth(20).copied())
            .map(|v| v as f32)
            .unwrap_or(curr);
        let trend = if curr > older + 0.2 { "↑" }
                    else if curr + 0.2 < older { "↓" }
                    else { "╌" };

        let fg = match curr {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };

        let hist: Vec<u64> = per_core_hist
            .deques
            .get(i)
            .map(|d| {
                let max_points = hchunks[0].width as usize;
                let start = d.len().saturating_sub(max_points);
                d.iter().skip(start).map(|&v| v as u64).collect()
            })
            .unwrap_or_default();

        let spark = Sparkline::default()
            .data(&hist)
            .max(100)
            .style(Style::default().fg(fg));
        f.render_widget(spark, hchunks[0]);

        let label = format!("cpu{:<2}{}{:>5.1}%", i, trend, curr);
        let line = Line::from(Span::styled(label, Style::default().fg(fg).add_modifier(Modifier::BOLD)));
        f.render_widget(Paragraph::new(line).right_aligned(), hchunks[1]);
    }
}