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
use crate::ui::theme::{
    PROCESS_SELECTION_BG, PROCESS_SELECTION_FG, PROCESS_TOOLTIP_BG, PROCESS_TOOLTIP_FG, SB_ARROW,
    SB_THUMB, SB_TRACK,
};
use crate::ui::util::human;

/// Simple fuzzy matching: returns true if all characters in needle appear in haystack in order (case-insensitive)
fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let mut haystack_chars = haystack_lower.chars();

    for needle_char in needle_lower.chars() {
        if !haystack_chars.any(|c| c == needle_char) {
            return false;
        }
    }
    true
}

/// Get filtered and sorted process indices based on search query and sort order
pub fn get_filtered_sorted_indices(
    metrics: &Metrics,
    search_query: &str,
    sort_by: ProcSortBy,
) -> Vec<usize> {
    // Filter processes by search query (fuzzy match)
    let mut filtered_idxs: Vec<usize> = if search_query.is_empty() {
        (0..metrics.top_processes.len()).collect()
    } else {
        (0..metrics.top_processes.len())
            .filter(|&i| fuzzy_match(&metrics.top_processes[i].name, search_query))
            .collect()
    };

    // Sort filtered rows
    match sort_by {
        ProcSortBy::CpuDesc => filtered_idxs.sort_by(|&a, &b| {
            let aa = metrics.top_processes[a].cpu_usage;
            let bb = metrics.top_processes[b].cpu_usage;
            bb.partial_cmp(&aa).unwrap_or(Ordering::Equal)
        }),
        ProcSortBy::MemDesc => filtered_idxs.sort_by(|&a, &b| {
            let aa = metrics.top_processes[a].mem_bytes;
            let bb = metrics.top_processes[b].mem_bytes;
            bb.cmp(&aa)
        }),
    }

    filtered_idxs
}

/// Parameters for drawing the top processes table
pub struct ProcessDisplayParams<'a> {
    pub metrics: Option<&'a Metrics>,
    pub scroll_offset: usize,
    pub sort_by: ProcSortBy,
    pub selected_process_pid: Option<u32>,
    pub selected_process_index: Option<usize>,
    pub search_query: &'a str,
    pub search_active: bool,
}

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

pub fn draw_top_processes(f: &mut ratatui::Frame<'_>, area: Rect, params: ProcessDisplayParams) {
    // Draw outer block and title
    let Some(mm) = params.metrics else { return };
    let total = mm.process_count.unwrap_or(mm.top_processes.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Top Processes ({total} total)"));
    f.render_widget(block, area);

    // Inner area (reserve space for search box if active)
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    // Draw search box if active
    let content_start_y = if params.search_active || !params.search_query.is_empty() {
        let search_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 3, // Height for border + content
        };

        let search_text = if params.search_active {
            format!("Search: {}_", params.search_query)
        } else {
            format!(
                "Filter: {} (press / to edit, c to clear)",
                params.search_query
            )
        };

        let search_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let search_paragraph = Paragraph::new(search_text)
            .block(search_block)
            .style(Style::default().fg(Color::Yellow));
        f.render_widget(search_paragraph, search_area);

        inner.y + 3
    } else {
        inner.y
    };

    // Content area (reserve 2 columns for scrollbar)
    let inner = Rect {
        x: inner.x,
        y: content_start_y,
        width: inner.width,
        height: inner.height.saturating_sub(content_start_y - (area.y + 1)),
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

    // Get filtered and sorted indices
    let idxs = get_filtered_sorted_indices(mm, params.search_query, params.sort_by);

    // Scrolling
    let total_rows = idxs.len();
    let header_rows = 1usize;
    let viewport_rows = content.height.saturating_sub(header_rows as u16) as usize;
    let max_off = total_rows.saturating_sub(viewport_rows);
    let offset = params.scroll_offset.min(max_off);
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

        let mut emphasis = if (cpu_val - peak_cpu).abs() < f32::EPSILON {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        // Check if this process is selected - prioritize PID matching
        let is_selected = if let Some(selected_pid) = params.selected_process_pid {
            selected_pid == p.pid
        } else if let Some(selected_idx) = params.selected_process_index {
            selected_idx == ix // ix is the absolute index in the sorted list
        } else {
            false
        };

        // Apply selection highlighting
        if is_selected {
            emphasis = emphasis
                .bg(PROCESS_SELECTION_BG)
                .fg(PROCESS_SELECTION_FG)
                .add_modifier(Modifier::BOLD);
        }

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
    let cpu_hdr = match params.sort_by {
        ProcSortBy::CpuDesc => "CPU % •",
        _ => "CPU %",
    };
    let mem_hdr = match params.sort_by {
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

    // Draw tooltip if a process is selected
    if let Some(selected_pid) = params.selected_process_pid {
        // Find the selected process to get its name
        let process_info = if let Some(metrics) = params.metrics {
            metrics
                .top_processes
                .iter()
                .find(|p| p.pid == selected_pid)
                .map(|p| format!("PID {} • {}", p.pid, p.name))
                .unwrap_or_else(|| format!("PID {selected_pid}"))
        } else {
            format!("PID {selected_pid}")
        };

        let tooltip_text = format!("{process_info} | Enter for details • X to unselect");
        let tooltip_width = tooltip_text.len() as u16 + 2; // Add padding
        let tooltip_height = 3;

        // Position tooltip at bottom-right of the processes area
        if area.width > tooltip_width + 2 && area.height > tooltip_height + 1 {
            let tooltip_area = Rect {
                x: area.x + area.width.saturating_sub(tooltip_width + 1),
                y: area.y + area.height.saturating_sub(tooltip_height + 1),
                width: tooltip_width,
                height: tooltip_height,
            };

            let tooltip_block = Block::default().borders(Borders::ALL).style(
                Style::default()
                    .bg(PROCESS_TOOLTIP_BG)
                    .fg(PROCESS_TOOLTIP_FG),
            );

            let tooltip_paragraph = Paragraph::new(tooltip_text)
                .block(tooltip_block)
                .wrap(ratatui::widgets::Wrap { trim: true });

            f.render_widget(tooltip_paragraph, tooltip_area);
        }
    }

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
/// Parameters for process key event handling
pub struct ProcessKeyParams<'a> {
    pub selected_process_pid: &'a mut Option<u32>,
    pub selected_process_index: &'a mut Option<usize>,
    pub key: crossterm::event::KeyEvent,
    pub metrics: Option<&'a Metrics>,
    pub sort_by: ProcSortBy,
    pub search_query: &'a str,
}

/// LEGACY: Use processes_handle_key_with_selection for enhanced functionality
#[allow(dead_code)]
pub fn processes_handle_key(
    scroll_offset: &mut usize,
    key: crossterm::event::KeyEvent,
    page_size: usize,
) {
    crate::ui::cpu::per_core_handle_key(scroll_offset, key, page_size);
}

pub fn processes_handle_key_with_selection(params: ProcessKeyParams) -> bool {
    use crossterm::event::KeyCode;

    match params.key.code {
        KeyCode::Up => {
            // Navigate through filtered and sorted results
            if let Some(m) = params.metrics {
                let idxs = get_filtered_sorted_indices(m, params.search_query, params.sort_by);

                if idxs.is_empty() {
                    // No filtered results, clear selection
                    *params.selected_process_index = None;
                    *params.selected_process_pid = None;
                } else if params.selected_process_index.is_none()
                    || params.selected_process_pid.is_none()
                {
                    // No selection - select the first process in filtered/sorted order
                    let first_idx = idxs[0];
                    *params.selected_process_index = Some(first_idx);
                    *params.selected_process_pid = Some(m.top_processes[first_idx].pid);
                } else if let Some(current_idx) = *params.selected_process_index {
                    // Find current position in filtered/sorted list
                    if let Some(pos) = idxs.iter().position(|&idx| idx == current_idx) {
                        if pos > 0 {
                            // Move up in filtered/sorted list
                            let new_idx = idxs[pos - 1];
                            *params.selected_process_index = Some(new_idx);
                            *params.selected_process_pid = Some(m.top_processes[new_idx].pid);
                        }
                    } else {
                        // Current selection not in filtered list, select first result
                        let first_idx = idxs[0];
                        *params.selected_process_index = Some(first_idx);
                        *params.selected_process_pid = Some(m.top_processes[first_idx].pid);
                    }
                }
            }
            true // Handled
        }
        KeyCode::Down => {
            // Navigate through filtered and sorted results
            if let Some(m) = params.metrics {
                let idxs = get_filtered_sorted_indices(m, params.search_query, params.sort_by);

                if idxs.is_empty() {
                    // No filtered results, clear selection
                    *params.selected_process_index = None;
                    *params.selected_process_pid = None;
                } else if params.selected_process_index.is_none()
                    || params.selected_process_pid.is_none()
                {
                    // No selection - select the first process in filtered/sorted order
                    let first_idx = idxs[0];
                    *params.selected_process_index = Some(first_idx);
                    *params.selected_process_pid = Some(m.top_processes[first_idx].pid);
                } else if let Some(current_idx) = *params.selected_process_index {
                    // Find current position in filtered/sorted list
                    if let Some(pos) = idxs.iter().position(|&idx| idx == current_idx) {
                        if pos + 1 < idxs.len() {
                            // Move down in filtered/sorted list
                            let new_idx = idxs[pos + 1];
                            *params.selected_process_index = Some(new_idx);
                            *params.selected_process_pid = Some(m.top_processes[new_idx].pid);
                        }
                    } else {
                        // Current selection not in filtered list, select first result
                        let first_idx = idxs[0];
                        *params.selected_process_index = Some(first_idx);
                        *params.selected_process_pid = Some(m.top_processes[first_idx].pid);
                    }
                }
            }
            true // Handled
        }
        KeyCode::Char('x') | KeyCode::Char('X') => {
            // Unselect any selected process
            if params.selected_process_pid.is_some() || params.selected_process_index.is_some() {
                *params.selected_process_pid = None;
                *params.selected_process_index = None;
                true // Handled
            } else {
                false // No selection to clear
            }
        }
        KeyCode::Enter => {
            // Signal that Enter was pressed with a selection
            params.selected_process_pid.is_some() // Return true if we have a selection to handle
        }
        _ => {
            // No other keys handled - let scrollbar handle all navigation
            false
        }
    }
}

/// Handle mouse for content scrolling and scrollbar dragging.
/// Returns Some(new_sort) if the header "CPU %" or "Mem" was clicked.
/// LEGACY: Use processes_handle_mouse_with_selection for enhanced functionality
#[allow(dead_code)]
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

/// Parameters for process mouse event handling
pub struct ProcessMouseParams<'a> {
    pub scroll_offset: &'a mut usize,
    pub selected_process_pid: &'a mut Option<u32>,
    pub selected_process_index: &'a mut Option<usize>,
    pub drag: &'a mut Option<crate::ui::cpu::PerCoreScrollDrag>,
    pub mouse: MouseEvent,
    pub area: Rect,
    pub total_rows: usize,
    pub metrics: Option<&'a Metrics>,
    pub sort_by: ProcSortBy,
    pub search_query: &'a str,
}

/// Enhanced mouse handler that also manages process selection
/// Returns Some(new_sort) if the header was clicked, or handles row selection
pub fn processes_handle_mouse_with_selection(params: ProcessMouseParams) -> Option<ProcSortBy> {
    // Inner and content areas (match draw_top_processes)
    let inner = Rect {
        x: params.area.x + 1,
        y: params.area.y + 1,
        width: params.area.width.saturating_sub(2),
        height: params.area.height.saturating_sub(2),
    };
    if inner.height == 0 || inner.width <= 2 {
        return None;
    }

    // Calculate content area - must match draw_top_processes exactly!
    // If search is active or query exists, content starts after search box (3 lines)
    let search_active = !params.search_query.is_empty();
    let content_start_y = if search_active { inner.y + 3 } else { inner.y };

    let content = Rect {
        x: inner.x,
        y: content_start_y,
        width: inner.width.saturating_sub(2),
        height: inner
            .height
            .saturating_sub(if search_active { 3 } else { 0 }),
    };

    // Scrollbar interactions (click arrows/page/drag)
    per_core_handle_scrollbar_mouse(
        params.scroll_offset,
        params.drag,
        params.mouse,
        params.area,
        params.total_rows,
    );

    // Wheel scrolling when inside the content
    crate::ui::cpu::per_core_handle_mouse(
        params.scroll_offset,
        params.mouse,
        content,
        content.height as usize,
    );

    // Header click to change sort
    let header_area = Rect {
        x: content.x,
        y: content.y,
        width: content.width,
        height: 1,
    };
    let inside_header = params.mouse.row == header_area.y
        && params.mouse.column >= header_area.x
        && params.mouse.column < header_area.x + header_area.width;

    if inside_header && matches!(params.mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        // Split header into the same columns
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(COLS.to_vec())
            .split(header_area);
        if params.mouse.column >= cols[2].x && params.mouse.column < cols[2].x + cols[2].width {
            return Some(ProcSortBy::CpuDesc);
        }
        if params.mouse.column >= cols[3].x && params.mouse.column < cols[3].x + cols[3].width {
            return Some(ProcSortBy::MemDesc);
        }
    }

    // Row click for process selection
    let data_start_row = content.y + 1; // Skip header
    let data_area_height = content.height.saturating_sub(1); // Exclude header

    if matches!(params.mouse.kind, MouseEventKind::Down(MouseButton::Left))
        && params.mouse.row >= data_start_row
        && params.mouse.row < data_start_row + data_area_height
        && params.mouse.column >= content.x
        && params.mouse.column < content.x + content.width
    {
        let clicked_row = (params.mouse.row - data_start_row) as usize;

        // Find the actual process using the same filtering/sorting logic as the drawing code
        if let Some(m) = params.metrics {
            // Use the same filtered and sorted indices as display
            let idxs = get_filtered_sorted_indices(m, params.search_query, params.sort_by);

            // Calculate which process was actually clicked based on filtered/sorted order
            let visible_process_position = *params.scroll_offset + clicked_row;
            if visible_process_position < idxs.len() {
                let actual_process_index = idxs[visible_process_position];
                let clicked_process = &m.top_processes[actual_process_index];
                *params.selected_process_pid = Some(clicked_process.pid);
                *params.selected_process_index = Some(actual_process_index);
            }
        }
    }

    // Clamp to valid range
    per_core_clamp(
        params.scroll_offset,
        params.total_rows,
        (content.height.saturating_sub(1)) as usize,
    );
    None
}
