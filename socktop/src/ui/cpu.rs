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
use crate::ui::fit::{cols, pick_pair};

/// Columns kept clear between the CPU title and the temperature readout.
const TITLE_GAP: u16 = 2;

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

    let (title, top_right_info) = cpu_title_for_width(
        m.map(|mm| mm.cpu_total),
        avg_cpu,
        m.and_then(|mm| mm.cpu_temp_c),
        area.width,
    );

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

    // Temperature overlays the top border, right-aligned inside the corner. The title
    // above is sized so the two cannot collide.
    if !top_right_info.is_empty() {
        let w = cols(&top_right_info);
        let info_area = Rect {
            x: area.x + area.width.saturating_sub(w + 1),
            y: area.y,
            width: w,
            height: 1,
        };
        let info_line = Line::from(Span::raw(top_right_info));
        f.render_widget(Paragraph::new(info_line), info_area);
    }
}

/// Health glyph for a CPU temperature.
fn temp_icon(t: f32) -> &'static str {
    if t < 50.0 {
        "😎"
    } else if t < 85.0 {
        "⚠️"
    } else {
        "🔥"
    }
}

/// Chooses the CPU pane's title and its right-aligned temperature readout for a pane
/// `width` columns wide.
///
/// Both are painted onto the pane's top border, so without a shared budget the
/// temperature simply overwrites the tail of the title on a narrow pane. Detail is given
/// up in this order: the `CPU Temp:` label, then the `now:`/`avg:` labels, then the
/// average reading, then the decimal on the temperature, and only last the temperature
/// itself — the readings are what the pane is for, but a thermal warning is worth more
/// than a second decimal place.
fn cpu_title_for_width(
    cpu_now: Option<f32>,
    avg_cpu: f64,
    temp_c: Option<f32>,
    width: u16,
) -> (String, String) {
    let Some(now) = cpu_now else {
        return ("CPU avg".into(), String::new());
    };

    // Two borders, plus a column of breathing room at each end of the title.
    let budget = width.saturating_sub(4);

    let labelled = format!("CPU (now: {now:>5.1}% | avg: {avg_cpu:>5.1}%)");
    let bare = format!("CPU ({now:.1}% | {avg_cpu:.1}%)");
    let now_only = format!("CPU ({now:.1}%)");

    let (temp_labelled, temp_plain, temp_coarse) = match temp_c {
        Some(t) => {
            let icon = temp_icon(t);
            (
                format!("CPU Temp: {t:.1}°C {icon}"),
                format!("{t:.1}°C {icon}"),
                format!("{t:.0}°C {icon}"),
            )
        }
        None => ("CPU Temp: N/A".into(), "N/A".into(), "N/A".into()),
    };

    let ladder = [
        (labelled.as_str(), temp_labelled.as_str()),
        (labelled.as_str(), temp_plain.as_str()),
        (bare.as_str(), temp_plain.as_str()),
        (bare.as_str(), temp_coarse.as_str()),
        (now_only.as_str(), temp_coarse.as_str()),
        (now_only.as_str(), ""),
    ];
    let (title, temp) = pick_pair(budget, TITLE_GAP, &ladder);
    (title.to_string(), temp.to_string())
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
mod title_tests {
    use super::*;

    /// The defect this replaces: the temperature was painted over the title's tail on a
    /// narrow pane. Whatever the width, the two must fit side by side on the border.
    #[test]
    fn title_and_temperature_never_overlap() {
        for width in 0..=200u16 {
            let (title, temp) = cpu_title_for_width(Some(3.4), 12.7, Some(43.0), width);
            let budget = width.saturating_sub(4);
            if temp.is_empty() {
                continue;
            }
            assert!(
                cols(&title) + cols(&temp) + TITLE_GAP <= budget,
                "width {width}: {title:?} + {temp:?} do not fit in {budget} columns"
            );
        }
    }

    /// The current CPU reading is the one thing the pane must always show.
    #[test]
    fn the_current_reading_always_survives() {
        for width in 20..=200u16 {
            let (title, _) = cpu_title_for_width(Some(3.4), 12.7, Some(43.0), width);
            assert!(
                title.contains("3.4"),
                "width {width}: lost the reading ({title:?})"
            );
        }
    }

    /// The ladder from the design: temp label, then now/avg labels, then the average,
    /// then the temperature's decimal, then the temperature.
    #[test]
    fn detail_is_dropped_in_priority_order() {
        let at = |w| cpu_title_for_width(Some(0.7), 1.3, Some(43.0), w);

        let (title, temp) = at(80);
        assert_eq!(title, "CPU (now:   0.7% | avg:   1.3%)");
        assert_eq!(temp, "CPU Temp: 43.0°C 😎");

        // The "CPU Temp:" label goes first; the readings keep their labels.
        let (title, temp) = at(50);
        assert_eq!(title, "CPU (now:   0.7% | avg:   1.3%)");
        assert_eq!(temp, "43.0°C 😎");

        // Then the now:/avg: labels.
        let (title, temp) = at(40);
        assert_eq!(title, "CPU (0.7% | 1.3%)");
        assert_eq!(temp, "43.0°C 😎");

        // Then the temperature's decimal.
        let (title, temp) = at(31);
        assert_eq!(title, "CPU (0.7% | 1.3%)");
        assert_eq!(temp, "43°C 😎");

        // Then the average reading.
        let (title, temp) = at(26);
        assert_eq!(title, "CPU (0.7%)");
        assert_eq!(temp, "43°C 😎");

        // Last of all, the temperature itself.
        let (title, temp) = at(15);
        assert_eq!(title, "CPU (0.7%)");
        assert_eq!(temp, "");
    }

    /// A hot CPU has to stay visible as a warning, so the glyph rides along with the
    /// reading at every tier that shows a temperature at all.
    #[test]
    fn the_thermal_glyph_tracks_the_temperature() {
        for (t, icon) in [(43.0, "😎"), (70.0, "⚠️"), (92.0, "🔥")] {
            for width in 26..=80u16 {
                let (_, temp) = cpu_title_for_width(Some(0.7), 1.3, Some(t), width);
                assert!(
                    temp.contains(icon),
                    "width {width} at {t}°C: expected {icon} in {temp:?}"
                );
            }
        }
    }

    /// An agent that reports no temperature must not leave a stray label behind.
    #[test]
    fn a_missing_temperature_degrades_to_nothing() {
        let (_, temp) = cpu_title_for_width(Some(0.7), 1.3, None, 80);
        assert_eq!(temp, "CPU Temp: N/A");
        let (_, temp) = cpu_title_for_width(Some(0.7), 1.3, None, 14);
        assert_eq!(temp, "");
    }

    /// Before the first payload arrives there are no readings to show.
    #[test]
    fn no_metrics_yet_shows_the_placeholder() {
        let (title, temp) = cpu_title_for_width(None, 0.0, None, 80);
        assert_eq!(title, "CPU avg");
        assert!(temp.is_empty());
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
            sampled_at_ms: None,
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
