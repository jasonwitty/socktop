//! CPU average sparkline + per-core mini bars.

use ratatui::style::Modifier;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind}; // + MouseButton

use crate::history::PerCoreHistory;
use crate::types::Metrics;

/// Subtle grey theme for the custom scrollbar
const SB_ARROW: Color = Color::Rgb(170,170,180);
const SB_TRACK: Color = Color::Rgb(170,170,180);
const SB_THUMB: Color = Color::Rgb(170,170,180);

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
        KeyCode::Up => *scroll_offset = scroll_offset.saturating_sub(1),
        KeyCode::Down => *scroll_offset = scroll_offset.saturating_add(1),
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
    let thumb_len = ((track * view + total - 1) / total).max(1).min(track);
    let top_for_offset = |off: usize| -> usize {
        if max_off == 0 {
            0
        } else {
            ((track - thumb_len) * off + max_off / 2) / max_off
        }
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
            if let Some(mut d) = drag.take() {
                if d.active {
                    let dy = (mouse.row as i32) - (d.start_y as i32);
                    let new_top = (d.start_top as i32 + dy)
                        .clamp(0, (track.saturating_sub(thumb_len)) as i32) as usize;
                    // Inverse mapping top -> offset
                    if track > thumb_len {
                        let denom = track - thumb_len;
                        offset = if max_off == 0 {
                            0
                        } else {
                            (new_top * max_off + denom / 2) / denom
                        };
                    } else {
                        offset = 0;
                    }
                    // Keep dragging
                    d.start_top = new_top;
                    d.start_y = mouse.row;
                    *drag = Some(d);
                }
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
pub fn draw_cpu_avg_graph(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    hist: &std::collections::VecDeque<u64>,
    m: Option<&Metrics>,
) {
    let title = if let Some(mm) = m {
        format!("CPU avg (now: {:>5.1}%)", mm.cpu_total)
    } else {
        "CPU avg".into()
    };
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

/// Draws the per-core CPU bars with sparklines and trends.
pub fn draw_per_core_bars(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    m: Option<&Metrics>,
    per_core_hist: &PerCoreHistory,
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
            .constraints([Constraint::Min(6), Constraint::Length(12)])
            .split(rect);

        let curr = mm.cpu_per_core[idx].clamp(0.0, 100.0);
        let older = per_core_hist
            .deques
            .get(idx)
            .and_then(|d| d.iter().rev().nth(20).copied())
            .map(|v| v as f32)
            .unwrap_or(curr);
        let trend = if curr > older + 0.2 { "↑" } else if curr + 0.2 < older { "↓" } else { "╌" };

        let fg = match curr {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };

        let hist: Vec<u64> = per_core_hist
            .deques
            .get(idx)
            .map(|d| {
                let max_points = hchunks[0].width as usize;
                let start = d.len().saturating_sub(max_points);
                d.iter().skip(start).map(|&v| v as u64).collect()
            })
            .unwrap_or_default();

        let spark = Sparkline::default().data(&hist).max(100).style(Style::default().fg(fg));
        f.render_widget(spark, hchunks[0]);

        let label = format!("cpu{:<2}{}{:>5.1}%", idx, trend, curr);
        let line = Line::from(Span::styled(
            label,
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ));
        f.render_widget(Paragraph::new(line).right_aligned(), hchunks[1]);
    }

    // Custom 1-col scrollbar with arrows, track, and exact mapping
    let scroll_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };
    if scroll_area.height >= 3 {
        let track = (scroll_area.height - 2) as usize;
        let total = total_rows.max(1);
        let view = viewport_rows.clamp(1, total);
        let max_off = total.saturating_sub(view);

        let thumb_len = ((track * view + total - 1) / total).max(1).min(track);
        let thumb_top = if max_off == 0 {
            0
        } else {
            ((track - thumb_len) * offset + max_off / 2) / max_off
        };

        // Build lines: top arrow, track (with thumb), bottom arrow
        let mut lines: Vec<Line> = Vec::with_capacity(scroll_area.height as usize);
        lines.push(Line::from(Span::styled("▲", Style::default().fg(SB_ARROW))));
        for i in 0..track {
            if i >= thumb_top && i < thumb_top + thumb_len {
                lines.push(Line::from(Span::styled("█", Style::default().fg(SB_THUMB))));
            } else {
                lines.push(Line::from(Span::styled("│", Style::default().fg(SB_TRACK))));
            }
        }
        lines.push(Line::from(Span::styled("▼", Style::default().fg(SB_ARROW))));

        f.render_widget(Paragraph::new(lines), scroll_area);
    }
}
