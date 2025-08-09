//! Top processes table with per-cell coloring and zebra striping.

use ratatui::style::Modifier;
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use crate::types::Metrics;
use crate::ui::util::human;

pub fn draw_top_processes(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let Some(mm) = m else {
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title("Top Processes"),
            area,
        );
        return;
    };

    let total_mem_bytes = mm.mem_total.max(1);
    let title = format!("Top Processes ({} total)", mm.process_count);
    let peak_cpu = mm
        .top_processes
        .iter()
        .map(|p| p.cpu_usage)
        .fold(0.0_f32, f32::max);

    let rows: Vec<Row> = mm
        .top_processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mem_pct = (p.mem_bytes as f64 / total_mem_bytes as f64) * 100.0;

            let cpu_fg = match p.cpu_usage {
                x if x < 25.0 => Color::Green,
                x if x < 60.0 => Color::Yellow,
                _ => Color::Red,
            };
            let mem_fg = match mem_pct {
                x if x < 5.0 => Color::Blue,
                x if x < 20.0 => Color::Magenta,
                _ => Color::Red,
            };

            let zebra = if i % 2 == 0 {
                Style::default().fg(Color::Gray)
            } else {
                Style::default()
            };

            let emphasis = if (p.cpu_usage - peak_cpu).abs() < f32::EPSILON {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(p.pid.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(p.name.clone()),
                Cell::from(format!("{:.1}%", p.cpu_usage)).style(Style::default().fg(cpu_fg)),
                Cell::from(human(p.mem_bytes)),
                Cell::from(format!("{:.2}%", mem_pct)).style(Style::default().fg(mem_fg)),
            ])
            .style(zebra.patch(emphasis))
        })
        .collect();

    let header = Row::new(vec!["PID", "Name", "CPU %", "Mem", "Mem %"]).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let table = Table::new(
        rows,
        vec![
            Constraint::Length(8),
            Constraint::Percentage(40),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}
