//! Top processes table with per-cell coloring, zebra striping, sorting, and a scrollbar.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Table},
};
use std::cmp::Ordering;

use crate::types::Metrics;
use crate::ui::cpu::{per_core_clamp, per_core_handle_scrollbar_mouse};
use crate::ui::theme::{SB_ARROW, SB_THUMB, SB_TRACK};
use crate::ui::util::human;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcSortBy {
    #[default]
    CpuDesc,
    MemDesc,
}

// Keep the original header widths here so drawing and hit-testing match.
const COLS: [Constraint; 5] = [
    Constraint::Length(8),      // PID
    Constraint::Percentage(40), // Name
    Constraint::Length(8),      // CPU %
    Constraint::Length(12),     // Mem
    Constraint::Length(8),      // Mem %
];

pub fn draw_top_processes(
    f: &mut ratatui::Frame<'_>,
    area: Rect,
    m: Option<&Metrics>,
    scroll_offset: usize,
    sort_by: ProcSortBy,
) {
    // Draw outer block and title
    let Some(mm) = m else { return };
    let total = mm.process_count.unwrap_or(mm.top_processes.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Top Processes ({total} total)"));
    f.render_widget(block, area);

    // Inner area and content area (reserve 2 columns for scrollbar)
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height < 1 || inner.width < 3 {
        return;
    }
    let content = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    // Sort rows (by CPU% or Mem bytes), descending.
    let mut idxs: Vec<usize> = (0..mm.top_processes.len()).collect();
    match sort_by {
        ProcSortBy::CpuDesc => idxs.sort_by(|&a, &b| {
            let aa = mm.top_processes[a].cpu_usage;
            let bb = mm.top_processes[b].cpu_usage;
            bb.partial_cmp(&aa).unwrap_or(Ordering::Equal)
        }),
        ProcSortBy::MemDesc => idxs.sort_by(|&a, &b| {
            let aa = mm.top_processes[a].mem_bytes;
            let bb = mm.top_processes[b].mem_bytes;
            bb.cmp(&aa)
        }),
    }

    // Scrolling
    let total_rows = idxs.len();
    let header_rows = 1usize;
    let viewport_rows = content.height.saturating_sub(header_rows as u16) as usize;
    let max_off = total_rows.saturating_sub(viewport_rows);
    let offset = scroll_offset.min(max_off);
    let show_n = total_rows.saturating_sub(offset).min(viewport_rows);

    // Build visible rows
    let total_mem_bytes = mm.mem_total.max(1);
    let peak_cpu = mm
        .top_processes
        .iter()
        .map(|p| p.cpu_usage)
        .fold(0.0_f32, f32::max);

    let rows_iter = idxs.iter().skip(offset).take(show_n).map(|&ix| {
        let p = &mm.top_processes[ix];
        let mem_pct = (p.mem_bytes as f64 / total_mem_bytes as f64) * 100.0;

        let cpu_val = p.cpu_usage;
        let cpu_fg = match cpu_val {
            x if x < 25.0 => Color::Green,
            x if x < 60.0 => Color::Yellow,
            _ => Color::Red,
        };
        let mem_fg = match mem_pct {
            x if x < 5.0 => Color::Blue,
            x if x < 20.0 => Color::Magenta,
            _ => Color::Red,
        };

        let emphasis = if (cpu_val - peak_cpu).abs() < f32::EPSILON {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let cpu_str = fmt_cpu_pct(cpu_val);

        ratatui::widgets::Row::new(vec![
            ratatui::widgets::Cell::from(p.pid.to_string())
                .style(Style::default().fg(Color::DarkGray)),
            ratatui::widgets::Cell::from(p.name.clone()),
            ratatui::widgets::Cell::from(cpu_str).style(Style::default().fg(cpu_fg)),
            ratatui::widgets::Cell::from(human(p.mem_bytes)),
            ratatui::widgets::Cell::from(format!("{mem_pct:.2}%"))
                .style(Style::default().fg(mem_fg)),
        ])
        .style(emphasis)
    });

    // Header with sort indicator
    let cpu_hdr = match sort_by {
        ProcSortBy::CpuDesc => "CPU % •",
        _ => "CPU %",
    };
    let mem_hdr = match sort_by {
        ProcSortBy::MemDesc => "Mem •",
        _ => "Mem",
    };
    let header = ratatui::widgets::Row::new(vec!["PID", "Name", cpu_hdr, mem_hdr, "Mem %"]).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    // Render table inside content area (no borders here; outer block already drawn)
    let table = Table::new(rows_iter, COLS.to_vec())
        .header(header)
        .column_spacing(1);
    f.render_widget(table, content);

    // Draw scrollbar like CPU pane
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

        let thumb_len = (track * view).div_ceil(total).max(1).min(track);
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

fn fmt_cpu_pct(v: f32) -> String {
    format!("{:>5.1}", v.clamp(0.0, 100.0))
}

/// Handle keyboard scrolling (Up/Down/PageUp/PageDown/Home/End)
pub fn processes_handle_key(
    scroll_offset: &mut usize,
    key: crossterm::event::KeyEvent,
    page_size: usize,
) {
    crate::ui::cpu::per_core_handle_key(scroll_offset, key, page_size);
}

/// Handle mouse for content scrolling and scrollbar dragging.
/// Returns Some(new_sort) if the header "CPU %" or "Mem" was clicked.
pub fn processes_handle_mouse(
    scroll_offset: &mut usize,
    drag: &mut Option<crate::ui::cpu::PerCoreScrollDrag>,
    mouse: MouseEvent,
    area: Rect,
    total_rows: usize,
) -> Option<ProcSortBy> {
    // Inner and content areas (match draw_top_processes)
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.height == 0 || inner.width <= 2 {
        return None;
    }
    let content = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    // Scrollbar interactions (click arrows/page/drag)
    per_core_handle_scrollbar_mouse(scroll_offset, drag, mouse, area, total_rows);

    // Wheel scrolling when inside the content
    crate::ui::cpu::per_core_handle_mouse(scroll_offset, mouse, content, content.height as usize);

    // Header click to change sort
    let header_area = Rect {
        x: content.x,
        y: content.y,
        width: content.width,
        height: 1,
    };
    let inside_header = mouse.row == header_area.y
        && mouse.column >= header_area.x
        && mouse.column < header_area.x + header_area.width;

    if inside_header && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        // Split header into the same columns
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(COLS.to_vec())
            .split(header_area);
        if mouse.column >= cols[2].x && mouse.column < cols[2].x + cols[2].width {
            return Some(ProcSortBy::CpuDesc);
        }
        if mouse.column >= cols[3].x && mouse.column < cols[3].x + cols[3].width {
            return Some(ProcSortBy::MemDesc);
        }
    }

    // Clamp to valid range
    per_core_clamp(
        scroll_offset,
        total_rows,
        (content.height.saturating_sub(1)) as usize,
    );
    None
}
