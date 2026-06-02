//! CPU average sparkline + per-core mini bars.

use crate::ui::theme::{SB_ARROW, SB_THUMB, SB_TRACK};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;
use ratatui::style::{Color, Style};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Sparkline,
    },
};

use crate::history::PerCoreHistory;
use crate::types::Metrics;

/// State for dragging the scrollbar thumb
#[derive(Clone, Copy, Debug, Default)]
pub struct PerCoreScrollDrag {
    pub active: bool,
    pub start_y: u16,     // mouse row where drag started
    pub start_top: usize, // thumb top (in track rows) at drag start
}

/// Returns the content area for per-core CPU bars, excluding borders and reserving space for scrollbar.
pub fn per_core_content_area(area: Rect) -> Rect {
    // Inner minus borders
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    // Reserve 1 column on the right for a gutter and 1 for the scrollbar
    Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    }
}

/// Handles key events for per-core CPU bars.
pub fn per_core_handle_key(scroll_offset: &mut usize, key: KeyEvent, page_size: usize) {
    match key.code {
        KeyCode::Left => *scroll_offset = scroll_offset.saturating_sub(1),
        KeyCode::Right => *scroll_offset = scroll_offset.saturating_add(1),
        KeyCode::PageUp => {
            let step = page_size.max(1);
            *scroll_offset = scroll_offset.saturating_sub(step);
        }
        KeyCode::PageDown => {
            let step = page_size.max(1);
            *scroll_offset = scroll_offset.saturating_add(step);
        }
        KeyCode::Home => *scroll_offset = 0,
        KeyCode::End => *scroll_offset = usize::MAX, // draw() clamps to max
        _ => {}
    }
}

/// Handles mouse wheel over the content.
pub fn per_core_handle_mouse(
    scroll_offset: &mut usize,
    mouse: MouseEvent,
    content_area: Rect,
    page_size: usize,
) {
    let inside = mouse.column >= content_area.x
        && mouse.column < content_area.x + content_area.width
        && mouse.row >= content_area.y
        && mouse.row < content_area.y + content_area.height;

    if !inside {
        return;
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => *scroll_offset = scroll_offset.saturating_sub(1),
        MouseEventKind::ScrollDown => *scroll_offset = scroll_offset.saturating_add(1),
        // Optional paging via horizontal wheel
        MouseEventKind::ScrollLeft => {
            let step = page_size.max(1);
            *scroll_offset = scroll_offset.saturating_sub(step);
        }
        MouseEventKind::ScrollRight => {
            let step = page_size.max(1);
            *scroll_offset = scroll_offset.saturating_add(step);
        }
        _ => {}
    }
}

/// Handles mouse interaction with the scrollbar itself (click arrows/page/drag).
pub fn per_core_handle_scrollbar_mouse(
    scroll_offset: &mut usize,
    drag: &mut Option<PerCoreScrollDrag>,
    mouse: MouseEvent,
    per_core_area: Rect,
    total_rows: usize,
) {
    // Geometry
    let inner = Rect {
        x: per_core_area.x + 1,
        y: per_core_area.y + 1,
        width: per_core_area.width.saturating_sub(2),
        height: per_core_area.height.saturating_sub(2),
    };
    if inner.height < 3 || inner.width < 1 {
        return;
    }
    let content = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };
    let scroll_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };
    let viewport_rows = content.height as usize;
    let total = total_rows.max(1);
    let view = viewport_rows.clamp(1, total);
    let max_off = total.saturating_sub(view);
    let mut offset = (*scroll_offset).min(max_off);

    // Track and current thumb
    let track = (scroll_area.height - 2) as usize;
    if track == 0 {
        return;
    }
    let thumb_len = (track * view).div_ceil(total).max(1).min(track);
    let top_for_offset = |off: usize| -> usize {
        ((track - thumb_len) * off + max_off / 2)
            .checked_div(max_off)
            .unwrap_or(0)
    };
    let thumb_top = top_for_offset(offset);

    let inside_scrollbar = mouse.column == scroll_area.x
        && mouse.row >= scroll_area.y
        && mouse.row < scroll_area.y + scroll_area.height;

    // Helper to page
    let page_up = || offset.saturating_sub(view.max(1));
    let page_down = || offset.saturating_add(view.max(1));

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) if inside_scrollbar => {
            // Where within the track?
            let row = mouse.row;
            if row == scroll_area.y {
                // Top arrow
                offset = offset.saturating_sub(1);
            } else if row + 1 == scroll_area.y + scroll_area.height {
                // Bottom arrow
                offset = offset.saturating_add(1);
            } else {
                // Inside track
                let rel = (row - (scroll_area.y + 1)) as usize;
                let thumb_end = thumb_top + thumb_len;
                if rel < thumb_top {
                    // Page up
                    offset = page_up();
                } else if rel >= thumb_end {
                    // Page down
                    offset = page_down();
                } else {
                    // Start dragging
                    *drag = Some(PerCoreScrollDrag {
                        active: true,
                        start_y: row,
                        start_top: thumb_top,
                    });
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(mut d) = drag.take()
                && d.active
            {
                let dy = (mouse.row as i32) - (d.start_y as i32);
                let new_top = (d.start_top as i32 + dy)
                    .clamp(0, (track.saturating_sub(thumb_len)) as i32)
                    as usize;
                // Inverse mapping top -> offset
                if track > thumb_len {
                    let denom = track - thumb_len;
                    offset = (new_top * max_off + denom / 2)
                        .checked_div(denom)
                        .unwrap_or(0);
                } else {
                    offset = 0;
                }
                // Keep dragging
                d.start_top = new_top;
                d.start_y = mouse.row;
                *drag = Some(d);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // End drag
            *drag = None;
        }
        // Also allow wheel scrolling when cursor is over the scrollbar
        MouseEventKind::ScrollUp if inside_scrollbar => {
            offset = offset.saturating_sub(1);
        }
        MouseEventKind::ScrollDown if inside_scrollbar => {
            offset = offset.saturating_add(1);
        }
        _ => {}
    }

    // Clamp and write back
    if offset > max_off {
        offset = max_off;
    }
    *scroll_offset = offset;
}

/// Clamp scroll offset to the valid range given content and viewport.
pub fn per_core_clamp(scroll_offset: &mut usize, total_rows: usize, viewport_rows: usize) {
    let max_offset = total_rows.saturating_sub(viewport_rows);
    if *scroll_offset > max_offset {
        *scroll_offset = max_offset;
    }
}

/// Draws the CPU average sparkline graph.
///
/// `hist_sum` is the running sum of `hist` maintained by the caller so we don't
/// fold the (up to 600-element) deque on every frame.
pub fn draw_cpu_avg_graph(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    hist: &mut std::collections::VecDeque<u64>,
    hist_sum: u64,
    m: Option<&Metrics>,
) {
    let avg_cpu = if hist.is_empty() {
        0.0
    } else {
        hist_sum as f64 / hist.len() as f64
    };

    let title = if let Some(mm) = m {
        format!("CPU (now: {:>5.1}% | avg: {:>5.1}%)", mm.cpu_total, avg_cpu)
    } else {
        "CPU avg".into()
    };

    // Build the top-right info (CPU temp and polling intervals)
    let top_right_info = if let Some(mm) = m {
        mm.cpu_temp_c
            .map(|t| {
                let icon = if t < 50.0 {
                    "😎"
                } else if t < 85.0 {
                    "⚠️"
                } else {
                    "🔥"
                };
                format!("CPU Temp: {t:.1}°C {icon}")
            })
            .unwrap_or_else(|| "CPU Temp: N/A".into())
    } else {
        String::new()
    };

    // Hand a slice directly to Sparkline. `make_contiguous` is amortized cheap
    // for our usage pattern (cap'd 600-element ring updated at 2 Hz) and lets
    // us skip the per-frame Vec allocation .collect() used to do.
    let max_points = area.width.saturating_sub(2) as usize;
    let start = hist.len().saturating_sub(max_points);
    let slice = &hist.make_contiguous()[start..];

    let spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(slice)
        .max(100)
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(spark, area);

    // Render the top-right info as text overlay in the top-right corner
    if !top_right_info.is_empty() {
        let info_area = Rect {
            x: area.x + area.width.saturating_sub(top_right_info.len() as u16 + 2),
            y: area.y,
            width: top_right_info.len() as u16 + 1,
            height: 1,
        };
        let info_line = Line::from(Span::raw(top_right_info));
        f.render_widget(Paragraph::new(info_line), info_area);
    }
}

/// Draws the per-core CPU bars with sparklines and trends.
pub fn draw_per_core_bars(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    m: Option<&Metrics>,
    per_core_hist: &mut PerCoreHistory,
    scroll_offset: usize,
) {
    f.render_widget(
        Block::default().borders(Borders::ALL).title("Per-core"),
        area,
    );
    let Some(mm) = m else {
        return;
    };

    // Compute inner rect and content area
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height == 0 || inner.width <= 2 {
        return;
    }
    let content = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    let total_rows = mm.cpu_per_core.len();
    let viewport_rows = content.height as usize;
    let max_offset = total_rows.saturating_sub(viewport_rows);
    let offset = scroll_offset.min(max_offset);
    let show_n = total_rows.saturating_sub(offset).min(viewport_rows);

    let constraints: Vec<Constraint> = (0..show_n).map(|_| Constraint::Length(1)).collect();
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(content);

    for i in 0..show_n {
        let idx = offset + i;
        let rect = vchunks[i];
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(6), Constraint::Length(13)])
            .split(rect);

        let curr = mm.cpu_per_core[idx].clamp(0.0, 100.0);
        let older = per_core_hist
            .deques
            .get(idx)
            .and_then(|d| d.iter().rev().nth(20).copied())
            .map(|v| v as f32)
            .unwrap_or(curr);

        // Trend indicator. Various Unicode glyphs we tried for the "flat"
        // trend (╌, ·) substituted as a hyphen on terminals with narrow font
        // coverage; combined with the next column being `100.0` they read as
        // `cpu0 -100.0%`, a nonsensical negative percent. Use a literal space
        // for the flat case — no character, no fallback, no confusion.
        let trend = if curr > older + 0.2 {
            "↑"
        } else if curr + 0.2 < older {
            "↓"
        } else {
            " "
        };

        let fg = match curr {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };

        // Borrow the per-core deque mutably so we can hand a contiguous slice
        // to Sparkline without allocating a fresh Vec each frame.
        if let Some(d) = per_core_hist.deques.get_mut(idx) {
            let max_points = hchunks[0].width as usize;
            let start = d.len().saturating_sub(max_points);
            let slice = &d.make_contiguous()[start..];
            let spark = Sparkline::default()
                .data(slice)
                .max(100)
                .style(Style::default().fg(fg));
            f.render_widget(spark, hchunks[0]);
        }

        // Hard space between the trend mark and the number — even if the
        // arrow glyphs (↑/↓) fall back to ASCII on a terminal that lacks
        // them, this space prevents the trend mark from visually joining
        // `100.0` to look like a negative value.
        let label = format!("cpu{idx:<2}{trend} {curr:>5.1}%");
        let line = Line::from(Span::styled(
            label,
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ));
        f.render_widget(Paragraph::new(line).right_aligned(), hchunks[1]);
    }

    // 1-col scrollbar (ratatui built-in widget). Skips drawing when the
    // content fits in the viewport, matching the previous behaviour.
    let scroll_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };
    let max_off = total_rows.saturating_sub(viewport_rows);
    if scroll_area.height >= 3 && max_off > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .thumb_symbol("█")
            .track_symbol(Some("│"))
            .thumb_style(Style::default().fg(SB_THUMB))
            .track_style(Style::default().fg(SB_TRACK))
            .begin_style(Style::default().fg(SB_ARROW))
            .end_style(Style::default().fg(SB_ARROW));
        let mut state = ScrollbarState::new(max_off).position(offset);
        f.render_stateful_widget(scrollbar, scroll_area, &mut state);
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use socktop_connector::Metrics;

    fn fake_metrics(cores: Vec<f32>) -> Metrics {
        Metrics {
            cpu_total: 0.0,
            cpu_per_core: cores,
            mem_total: 1024,
            mem_used: 0,
            swap_total: 0,
            swap_used: 0,
            hostname: "t".into(),
            cpu_temp_c: None,
            disks: vec![],
            networks: vec![],
            top_processes: vec![],
            gpus: None,
            process_count: Some(0),
        }
    }

    fn dump(terminal: &Terminal<TestBackend>) -> String {
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

    /// Regression: the "flat" trend glyph used to be `╌` (U+254C), then `·`
    /// (U+00B7) — both substituted as a hyphen on terminals with narrow font
    /// coverage. When a core sat at exactly 100% the label rendered as
    /// `cpu3 -100.0%` (no space between trend and digits). Now we use a
    /// literal space for the flat case AND insert a hard space between every
    /// trend mark and the number, so no glyph substitution can produce a
    /// "-100" substring. We assert that across flat AND transitioning cores.
    #[test]
    fn percore_label_never_renders_as_negative() {
        let m = fake_metrics(vec![100.0, 100.0, 100.0, 100.0]);
        let mut hist = PerCoreHistory::new(60);
        hist.ensure_cores(4);
        // First sample: history is empty, no trend on first frame.
        hist.push_samples(&m.cpu_per_core);
        // Second sample: identical values → flat trend (the user's complaint).
        hist.push_samples(&m.cpu_per_core);

        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_per_core_bars(f, Rect::new(0, 0, 120, 8), Some(&m), &mut hist, 0);
            })
            .unwrap();

        let out = dump(&terminal);
        eprintln!("---flat 100% render---\n{out}");
        assert!(!out.contains("-100"), "found '-100' in flat-trend render");

        // Decreasing trend at saturation: hist was high, current drops a bit.
        let mut hist2 = PerCoreHistory::new(60);
        hist2.ensure_cores(4);
        for _ in 0..25 {
            hist2.push_samples(&[100.0, 100.0, 100.0, 100.0]);
        }
        let m2 = fake_metrics(vec![100.0, 100.0, 100.0, 80.0]);
        hist2.push_samples(&m2.cpu_per_core);

        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw_per_core_bars(f, Rect::new(0, 0, 120, 8), Some(&m2), &mut hist2, 0);
            })
            .unwrap();

        let out = dump(&terminal);
        eprintln!("---decreasing render---\n{out}");
        assert!(
            !out.contains("-100"),
            "found '-100' in decreasing-trend render"
        );
        assert!(
            !out.contains("-80"),
            "found '-80' in decreasing-trend render"
        );
    }
}
