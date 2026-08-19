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
        let name_text = g.name.as_deref().unwrap_or("GPU");
        let name_p = Paragraph::new(Span::raw(name_text)).style(Style::default().fg(Color::Gray));
        f.render_widget(name_p, rows[i * 3]);

        // Row 2: Utilization bar + right label
        let util_cols = split_bar(rows[i * 3 + 1]);
        let util = g.utilization.unwrap_or(0.0).clamp(0.0, 100.0) as u16;
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
        let used = g.mem_used.unwrap_or(0);
        let total = g.mem_total.unwrap_or(1);
        let mem_ratio = used as f64 / total as f64;
        let mem_pct = (mem_ratio * 100.0).round() as u16;

        let mem_gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::LightMagenta))
            .label(Span::raw(""))
            .ratio(mem_ratio);
        f.render_widget(mem_gauge, mem_cols[0]);
        let used_s = fmt_bytes(used);
        let total_s = fmt_bytes(total);
        f.render_widget(
            Paragraph::new(Span::raw(format!("vram: {used_s}/{total_s} ({mem_pct}%)")))
                .style(Style::default().fg(Color::Gray)),
            mem_cols[1],
        );
    }
}

/// One-line GPU strip for compact mode: no device name (it is the first thing to lose
/// value when rows are scarce), just utilisation and VRAM on the single content row
/// between the block borders. Only the first GPU fits; the title says so when there are
/// more.
pub fn draw_gpu_compact(f: &mut ratatui::Frame<'_>, area: Rect, m: Option<&Metrics>) {
    let gpus = m.and_then(|mm| mm.gpus.as_ref());
    let count = gpus.map(|g| g.len()).unwrap_or(0);
    let title = if count > 1 {
        format!("GPU (1/{count})")
    } else {
        "GPU".to_string()
    };
    f.render_widget(Block::default().borders(Borders::ALL).title(title), area);

    if area.height < 3 || area.width <= 2 {
        return;
    }
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: 1,
    };

    let Some(g) = gpus.and_then(|v| v.first()) else {
        f.render_widget(Paragraph::new("No GPUs"), inner);
        return;
    };

    let util = g.utilization.unwrap_or(0.0).clamp(0.0, 100.0) as u16;
    let used = g.mem_used.unwrap_or(0);
    let total = g.mem_total.unwrap_or(1);
    let mem_ratio = if total > 0 {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let util_label = format!("util: {util}%");
    let mem_label = format!(
        "vram: {}/{} ({}%)",
        fmt_bytes(used),
        fmt_bytes(total),
        (mem_ratio * 100.0).round() as u16
    );

    // Bars are sized explicitly rather than left to stretch: an idle bar renders as
    // empty cells, so a full-width one turns into a long blank run between two labels.
    const MIN_GAUGE_W: u16 = 6;
    const MAX_GAUGE_W: u16 = 24;
    let labels_w = util_label.len() as u16 + mem_label.len() as u16 + 4; // one space each side
    let gauge_w = inner
        .width
        .saturating_sub(labels_w)
        .min(2 * MAX_GAUGE_W)
        .div_euclid(2);

    // Too narrow for bars worth drawing: keep the numbers, drop the bars.
    if gauge_w < MIN_GAUGE_W {
        f.render_widget(
            Paragraph::new(Span::raw(format!("{util_label}  {mem_label}")))
                .style(Style::default().fg(Color::Gray)),
            inner,
        );
        return;
    }

    // Each label leads its own bar. Bar-then-label (as the tall panel does) is ambiguous
    // on a single line: with an idle bar rendering empty, the next pair's fill ends up
    // flush against the previous pair's text and reads as belonging to it.
    let mut x = inner.x;
    let mut place = |w: u16| {
        let r = Rect {
            x,
            y: inner.y,
            width: w,
            height: 1,
        };
        x += w;
        r
    };
    let util_rect = place(util_label.len() as u16 + 2);
    let util_bar = place(gauge_w);
    let mem_rect = place(mem_label.len() as u16 + 2);
    let mem_bar = place(gauge_w);

    let label = |text: &str| {
        Paragraph::new(Span::raw(format!(" {text} "))).style(Style::default().fg(Color::Gray))
    };

    f.render_widget(label(&util_label), util_rect);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Green))
            .label(Span::raw(""))
            .ratio(util as f64 / 100.0),
        util_bar,
    );
    f.render_widget(label(&mem_label), mem_rect);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(Color::LightMagenta))
            .label(Span::raw(""))
            .ratio(mem_ratio),
        mem_bar,
    );
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use socktop_connector::{GpuInfo, Metrics};

    fn gpu(name: &str) -> GpuInfo {
        GpuInfo {
            name: Some(name.into()),
            vendor: None,
            utilization: Some(42.0),
            mem_used: Some(4_724_464_025),
            mem_total: Some(17_070_817_280),
            temp: None,
        }
    }

    fn metrics(gpus: Option<Vec<GpuInfo>>) -> Metrics {
        Metrics {
            cpu_total: 0.0,
            cpu_per_core: vec![],
            mem_total: 1024,
            mem_used: 0,
            swap_total: 0,
            swap_used: 0,
            hostname: "t".into(),
            cpu_temp_c: None,
            disks: vec![],
            networks: vec![],
            top_processes: vec![],
            gpus,
            process_count: Some(0),
        }
    }

    fn render(width: u16, m: &Metrics) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
        terminal
            .draw(|f| draw_gpu_compact(f, Rect::new(0, 0, width, 3), Some(m)))
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area().height {
            for x in 0..buf.area().width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Compact mode drops the device name — the row is one line and the numbers are
    /// what the space is for.
    #[test]
    fn compact_strip_omits_the_device_name() {
        let m = metrics(Some(vec![gpu("NVIDIA GeForce RTX 5080")]));
        let out = render(80, &m);
        assert!(
            !out.contains("NVIDIA"),
            "name leaked into compact strip:\n{out}"
        );
        assert!(out.contains("util: 42%"), "{out}");
        assert!(out.contains("vram: 4.4G/15.9G (28%)"), "{out}");
    }

    /// A second GPU cannot fit on one line, so the title has to say the strip is partial
    /// rather than silently showing only the first card.
    #[test]
    fn multiple_gpus_are_flagged_in_the_title() {
        let one = render(80, &metrics(Some(vec![gpu("a")])));
        assert!(one.contains("GPU") && !one.contains("1/"), "{one}");

        let two = render(80, &metrics(Some(vec![gpu("a"), gpu("b")])));
        assert!(two.contains("GPU (1/2)"), "{two}");
    }

    /// Narrow terminals drop the gauges rather than rendering two-cell stubs, but must
    /// never drop the numbers.
    #[test]
    fn narrow_strip_keeps_the_numbers() {
        let m = metrics(Some(vec![gpu("a")]));
        for width in [20u16, 30, 40, 47, 48, 80, 200] {
            let out = render(width, &m);
            if width >= 40 {
                assert!(out.contains("util: 42%"), "width {width}:\n{out}");
            }
            // No panic, and the block always closes on the last row.
            assert_eq!(out.lines().count(), 3, "width {width}");
        }
    }

    #[test]
    fn missing_gpu_payload_does_not_panic() {
        assert!(render(80, &metrics(None)).contains("No GPUs"));
        assert!(render(80, &metrics(Some(vec![]))).contains("No GPUs"));
    }
}
