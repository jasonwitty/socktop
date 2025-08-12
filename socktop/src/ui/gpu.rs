use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Gauge, Paragraph},
};

use crate::types::Metrics;

fn fmt_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let fb = b as f64;

    if fb >= GB {
        format!("{:.1}G", fb / GB)
    } else if fb >= MB {
        format!("{:.1}M", fb / MB)
    } else if fb >= KB {
        format!("{:.1}K", fb / KB)
    } else {
        format!("{b}B")
    }
}

pub fn draw_gpu(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let mut area = area;
    let block = Block::default().borders(Borders::ALL).title("GPU");
    f.render_widget(block, area);

    // Guard: need some space inside the block
    if area.height <= 2 || area.width <= 2 {
        return;
    }

    // Inner padding consistent with the rest of the app
    area.y += 1;
    area.height = area.height.saturating_sub(2);
    area.x += 1;
    area.width = area.width.saturating_sub(2);

    let Some(metrics) = m else {
        return;
    };

    let Some(gpus) = metrics.gpus.as_ref() else {
        f.render_widget(Paragraph::new("No GPUs"), area);
        return;
    };
    if gpus.is_empty() {
        f.render_widget(Paragraph::new("No GPUs"), area);
        return;
    }

    // Show 3 rows per GPU: name, util bar, vram bar.
    if area.height < 3 {
        return;
    }
    let per_gpu_rows: u16 = 3;
    let max_gpus = (area.height / per_gpu_rows) as usize;
    let count = gpus.len().min(max_gpus);

    let constraints = vec![Constraint::Length(1); count * per_gpu_rows as usize];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Per bar horizontal layout: [gauge] [value]
    let split_bar = |r: Rect| {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(8),     // gauge column
                Constraint::Length(24), // value column
            ])
            .split(r)
    };

    for i in 0..count {
        let g = &gpus[i];

        // Row 1: GPU name
        let name_text = g.name.clone();
        f.render_widget(
            Paragraph::new(Span::raw(name_text)).style(Style::default().fg(Color::Gray)),
            rows[i * 3],
        );

        // Row 2: Utilization bar + right label
        let util_cols = split_bar(rows[i * 3 + 1]);
        let util = g.utilization_gpu_pct.min(100) as u16;
        let util_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Green))
            .label(Span::raw(""))
            .ratio(util as f64 / 100.0);
        f.render_widget(util_gauge, util_cols[0]);
        f.render_widget(
            Paragraph::new(Span::raw(format!("util: {util}%")))
                .style(Style::default().fg(Color::Gray)),
            util_cols[1],
        );

        // Row 3: VRAM bar + right label
        let mem_cols = split_bar(rows[i * 3 + 2]);
        let used = g.mem_used_bytes;
        let total = g.mem_total_bytes.max(1);
        let mem_ratio = used as f64 / total as f64;
        let mem_pct = (mem_ratio * 100.0).round() as u16;

        let mem_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::LightMagenta))
            .label(Span::raw(""))
            .ratio(mem_ratio);
        f.render_widget(mem_gauge, mem_cols[0]);
        // Prepare strings to enable captured identifiers in format!
        let used_s = fmt_bytes(used);
        let total_s = fmt_bytes(total);
        f.render_widget(
            Paragraph::new(Span::raw(format!("vram: {used_s}/{total_s} ({mem_pct}%)")))
                .style(Style::default().fg(Color::Gray)),
            mem_cols[1],
        );
    }
}