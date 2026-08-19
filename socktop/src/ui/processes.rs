//! Top processes table with per-cell coloring, zebra striping, sorting, and a scrollbar.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Modifier;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Table},
};
use std::cmp::Ordering;

use crate::types::Metrics;
use crate::ui::cpu::{per_core_clamp, per_core_handle_scrollbar_mouse};
use crate::ui::theme::{
    PROCESS_SELECTION_BG, PROCESS_SELECTION_FG, PROCESS_TOOLTIP_BG, PROCESS_TOOLTIP_FG, SB_ARROW,
    SB_THUMB, SB_TRACK,
};

/// Simple fuzzy matching: returns true if all characters in needle appear in
/// haystack in order, ASCII-case-insensitive. Lowercase normalization is done
/// on the fly so we don't allocate two `String`s per haystack like the old
/// version did (this runs once per process per frame).
fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut haystack_chars = haystack.chars().map(|c| c.to_ascii_lowercase());
    for needle_char in needle.chars().map(|c| c.to_ascii_lowercase()) {
        if !haystack_chars.any(|c| c == needle_char) {
            return false;
        }
    }
    true
}

/// Fill `out` with filtered + sorted process indices. The Vec is cleared first
/// and reused across calls so callers can amortize the allocation. This is
/// the underlying helper for the App-side cached slice.
pub fn fill_filtered_sorted_indices(
    metrics: &Metrics,
    search_query: &str,
    sort_by: ProcSortBy,
    out: &mut Vec<usize>,
) {
    out.clear();
    out.reserve(metrics.top_processes.len());
    if search_query.is_empty() {
        out.extend(0..metrics.top_processes.len());
    } else {
        out.extend(
            (0..metrics.top_processes.len())
                .filter(|&i| fuzzy_match(&metrics.top_processes[i].name, search_query)),
        );
    }
    match sort_by {
        ProcSortBy::CpuDesc => out.sort_by(|&a, &b| {
            let aa = metrics.top_processes[a].cpu_usage;
            let bb = metrics.top_processes[b].cpu_usage;
            bb.partial_cmp(&aa).unwrap_or(Ordering::Equal)
        }),
        ProcSortBy::MemDesc => out.sort_by(|&a, &b| {
            let aa = metrics.top_processes[a].mem_bytes;
            let bb = metrics.top_processes[b].mem_bytes;
            bb.cmp(&aa)
        }),
    }
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
    /// Precomputed filtered + sorted indices into `metrics.top_processes`.
    /// Maintained on the App side so the draw path never recomputes the list.
    pub filtered_indices: &'a [usize],
    /// Pre-formatted strings for each row of `metrics.top_processes`.
    /// Indexed the same as `metrics.top_processes`. Empty when no procs poll
    /// has run yet (the draw path falls back to fast inline formatting).
    pub cached_rows: &'a [CachedRow],
    /// Peak cpu_usage from the most recent cache build; used to bold the
    /// busiest process. -1.0 if no cache.
    pub peak_cpu: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcSortBy {
    #[default]
    CpuDesc,
    MemDesc,
}

/// Pre-formatted strings for one row of the process table. Built once per
/// `Processes` poll (cadence ~2s) and reused by every draw frame in between
/// so the diff renderer can suppress repaints when nothing changed.
#[derive(Debug, Clone)]
pub struct CachedRow {
    pub pid_str: String,
    pub cpu_str: String,
    pub mem_str: String,
    pub mem_pct_str: String,
    pub mem_pct: f64,
    pub cpu_val: f32,
}

/// Build a fresh row cache parallel to `metrics.top_processes`. Reuses `out`'s
/// allocation when possible. Also returns the peak cpu_usage observed, which
/// the draw path uses to bold the busiest process.
pub fn rebuild_row_cache(metrics: &Metrics, out: &mut Vec<CachedRow>) -> f32 {
    out.clear();
    out.reserve(metrics.top_processes.len());
    let total = metrics.mem_total.max(1);
    let mut peak = 0.0_f32;
    for p in &metrics.top_processes {
        let mem_pct = (p.mem_bytes as f64 / total as f64) * 100.0;
        let cpu_val = p.cpu_usage;
        if cpu_val > peak {
            peak = cpu_val;
        }
        out.push(CachedRow {
            pid_str: p.pid.to_string(),
            cpu_str: format!("{:>5.1}", cpu_val.clamp(0.0, 100.0)),
            mem_str: crate::ui::util::human(p.mem_bytes),
            mem_pct_str: format!("{mem_pct:.2}%"),
            mem_pct,
            cpu_val,
        });
    }
    peak
}

const PID_W: u16 = 8;
const CPU_W: u16 = 8;
const MEM_W: u16 = 12;
const MEM_PCT_W: u16 = 8;
/// Columns the Name field needs to identify anything. Every other column is only added
/// once Name already has this much, so Name can no longer be squeezed to nothing.
const NAME_MIN_W: u16 = 8;
/// `Table::column_spacing`.
const COL_SPACING: u16 = 1;

/// Which process columns fit in the pane, and where they sit.
///
/// The table used to hand the layout solver a fixed, over-constrained set, so on a narrow
/// pane the solver crushed the percentage-sized Name column to nothing while the fixed
/// PID and Mem % columns kept their full width — losing the one field that identifies the
/// process while keeping the ones that do not.
///
/// Columns are now added in priority order as the pane widens, so they are shed in
/// reverse as it narrows: Name is unconditional, then CPU %, then Mem, then PID, and
/// Mem % last (it is derivable from Mem, so it is the least costly to lose).
///
/// Both the draw path and the header-click hit-testing build this from the same width, so
/// a sort click always lands on the column the user can actually see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcColumns {
    pub pid: bool,
    pub cpu: bool,
    pub mem: bool,
    pub mem_pct: bool,
}

impl ProcColumns {
    pub fn for_width(width: u16) -> Self {
        // Each tier is the previous one plus a column and the gap before it.
        let with_cpu = NAME_MIN_W + COL_SPACING + CPU_W;
        let with_mem = with_cpu + COL_SPACING + MEM_W;
        let with_pid = with_mem + COL_SPACING + PID_W;
        let with_mem_pct = with_pid + COL_SPACING + MEM_PCT_W;
        Self {
            cpu: width >= with_cpu,
            mem: width >= with_mem,
            pid: width >= with_pid,
            mem_pct: width >= with_mem_pct,
        }
    }

    /// Column constraints in render order. Name takes whatever the others leave.
    pub fn constraints(&self) -> Vec<Constraint> {
        let mut c = Vec::with_capacity(5);
        if self.pid {
            c.push(Constraint::Length(PID_W));
        }
        c.push(Constraint::Fill(1)); // Name
        if self.cpu {
            c.push(Constraint::Length(CPU_W));
        }
        if self.mem {
            c.push(Constraint::Length(MEM_W));
        }
        if self.mem_pct {
            c.push(Constraint::Length(MEM_PCT_W));
        }
        c
    }

    /// Position of the CPU % column, which is clickable to sort. `None` when too narrow
    /// to render it.
    pub fn cpu_index(&self) -> Option<usize> {
        self.cpu.then(|| 1 + usize::from(self.pid))
    }

    /// Position of the Mem column, which is clickable to sort.
    pub fn mem_index(&self) -> Option<usize> {
        self.mem
            .then(|| 1 + usize::from(self.pid) + usize::from(self.cpu))
    }
}

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

    let idxs = params.filtered_indices;

    // Scrolling
    let total_rows = idxs.len();
    let header_rows = 1usize;
    let viewport_rows = content.height.saturating_sub(header_rows as u16) as usize;
    let max_off = total_rows.saturating_sub(viewport_rows);
    let offset = params.scroll_offset.min(max_off);
    let show_n = total_rows.saturating_sub(offset).min(viewport_rows);

    // Use the App-side cache when available so we avoid allocating ~5 strings
    // per row every frame. Falls back to inline formatting (slow path) when
    // the cache hasn't been built yet — e.g. the very first frame before the
    // initial procs poll completes.
    let cache_ok = params.cached_rows.len() == mm.top_processes.len();
    let total_mem_bytes = mm.mem_total.max(1);
    let peak_cpu = if cache_ok {
        params.peak_cpu
    } else {
        mm.top_processes
            .iter()
            .map(|p| p.cpu_usage)
            .fold(0.0_f32, f32::max)
    };

    let columns = ProcColumns::for_width(content.width);

    let rows_iter = idxs.iter().skip(offset).take(show_n).map(|&ix| {
        let p = &mm.top_processes[ix];

        let (
            cpu_val,
            mem_pct,
            pid_span,
            name_span,
            cpu_span_text,
            mem_span_text,
            mem_pct_span_text,
        ) = if cache_ok {
            let row = &params.cached_rows[ix];
            (
                row.cpu_val,
                row.mem_pct,
                Span::raw(row.pid_str.as_str()),
                Span::raw(p.name.as_str()),
                row.cpu_str.as_str(),
                row.mem_str.as_str(),
                row.mem_pct_str.as_str(),
            )
        } else {
            let mem_pct = (p.mem_bytes as f64 / total_mem_bytes as f64) * 100.0;
            // SLOW path: only the very first frame before the cache exists.
            // We leak the formatted strings via Box::leak'd statics? No —
            // simpler: emit empty placeholders. Cache will exist within
            // ~500ms and the diff renderer fills it in.
            (
                p.cpu_usage,
                mem_pct,
                Span::raw(""),
                Span::raw(""),
                "",
                "",
                "",
            )
        };

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

        let is_selected = if let Some(selected_pid) = params.selected_process_pid {
            selected_pid == p.pid
        } else if let Some(selected_idx) = params.selected_process_index {
            selected_idx == ix
        } else {
            false
        };

        if is_selected {
            emphasis = emphasis
                .bg(PROCESS_SELECTION_BG)
                .fg(PROCESS_SELECTION_FG)
                .add_modifier(Modifier::BOLD);
        }

        let mut cells = Vec::with_capacity(5);
        if columns.pid {
            cells.push(
                ratatui::widgets::Cell::from(pid_span).style(Style::default().fg(Color::DarkGray)),
            );
        }
        cells.push(ratatui::widgets::Cell::from(name_span));
        if columns.cpu {
            cells.push(
                ratatui::widgets::Cell::from(Span::raw(cpu_span_text))
                    .style(Style::default().fg(cpu_fg)),
            );
        }
        if columns.mem {
            cells.push(ratatui::widgets::Cell::from(Span::raw(mem_span_text)));
        }
        if columns.mem_pct {
            cells.push(
                ratatui::widgets::Cell::from(Span::raw(mem_pct_span_text))
                    .style(Style::default().fg(mem_fg)),
            );
        }
        ratatui::widgets::Row::new(cells).style(emphasis)
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
    let mut header_cells = Vec::with_capacity(5);
    if columns.pid {
        header_cells.push("PID");
    }
    header_cells.push("Name");
    if columns.cpu {
        header_cells.push(cpu_hdr);
    }
    if columns.mem {
        header_cells.push(mem_hdr);
    }
    if columns.mem_pct {
        header_cells.push("Mem %");
    }
    let header = ratatui::widgets::Row::new(header_cells).style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    // Render table inside content area (no borders here; outer block already drawn)
    let table = Table::new(rows_iter, columns.constraints())
        .header(header)
        .column_spacing(COL_SPACING);
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

    // Scrollbar (ratatui built-in). Skip drawing when content fits in viewport.
    let scroll_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };
    let max_off_for_bar = total_rows.saturating_sub(viewport_rows);
    if scroll_area.height >= 3 && max_off_for_bar > 0 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .thumb_symbol("█")
            .track_symbol(Some("│"))
            .thumb_style(Style::default().fg(SB_THUMB))
            .track_style(Style::default().fg(SB_TRACK))
            .begin_style(Style::default().fg(SB_ARROW))
            .end_style(Style::default().fg(SB_ARROW));
        let mut state = ScrollbarState::new(max_off_for_bar).position(offset);
        f.render_stateful_widget(scrollbar, scroll_area, &mut state);
    }
}

/// Parameters for process key event handling
pub struct ProcessKeyParams<'a> {
    pub selected_process_pid: &'a mut Option<u32>,
    pub selected_process_index: &'a mut Option<usize>,
    pub key: crossterm::event::KeyEvent,
    pub metrics: Option<&'a Metrics>,
    pub filtered_indices: &'a [usize],
}

pub fn processes_handle_key_with_selection(params: ProcessKeyParams) -> bool {
    use crossterm::event::KeyCode;

    let move_selection = |delta: isize,
                          sel_idx: &mut Option<usize>,
                          sel_pid: &mut Option<u32>,
                          metrics: Option<&Metrics>,
                          idxs: &[usize]| {
        let Some(m) = metrics else { return };
        if idxs.is_empty() {
            *sel_idx = None;
            *sel_pid = None;
            return;
        }
        if sel_idx.is_none() || sel_pid.is_none() {
            let first_idx = idxs[0];
            *sel_idx = Some(first_idx);
            *sel_pid = Some(m.top_processes[first_idx].pid);
            return;
        }
        let current_idx = sel_idx.unwrap();
        match idxs.iter().position(|&idx| idx == current_idx) {
            Some(pos) => {
                let new_pos = (pos as isize + delta).clamp(0, idxs.len() as isize - 1) as usize;
                if new_pos != pos {
                    let new_idx = idxs[new_pos];
                    *sel_idx = Some(new_idx);
                    *sel_pid = Some(m.top_processes[new_idx].pid);
                }
            }
            None => {
                // Current selection no longer in filtered list
                let first_idx = idxs[0];
                *sel_idx = Some(first_idx);
                *sel_pid = Some(m.top_processes[first_idx].pid);
            }
        }
    };

    match params.key.code {
        KeyCode::Up => {
            move_selection(
                -1,
                params.selected_process_index,
                params.selected_process_pid,
                params.metrics,
                params.filtered_indices,
            );
            true
        }
        KeyCode::Down => {
            move_selection(
                1,
                params.selected_process_index,
                params.selected_process_pid,
                params.metrics,
                params.filtered_indices,
            );
            true
        }
        KeyCode::Char('x') | KeyCode::Char('X')
            if params.selected_process_pid.is_some() || params.selected_process_index.is_some() =>
        {
            *params.selected_process_pid = None;
            *params.selected_process_index = None;
            true
        }
        KeyCode::Char('x') | KeyCode::Char('X') => false,
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
    /// True when the on-screen search box is currently being drawn (active
    /// edit mode OR a non-empty filter is showing). The caller computes this
    /// from the same condition as the draw path.
    pub search_box_visible: bool,
    pub filtered_indices: &'a [usize],
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
    // If a search box is being drawn (active edit mode OR a filter showing),
    // content starts 3 rows below.
    let content_start_y = if params.search_box_visible {
        inner.y + 3
    } else {
        inner.y
    };

    let content = Rect {
        x: inner.x,
        y: content_start_y,
        width: inner.width.saturating_sub(2),
        height: inner
            .height
            .saturating_sub(if params.search_box_visible { 3 } else { 0 }),
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
        // Split the header the same way the draw path did, so a click lands on the
        // column actually on screen even when PID has been dropped.
        let columns = ProcColumns::for_width(header_area.width);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(columns.constraints())
            .spacing(COL_SPACING) // must match Table::column_spacing in the draw path
            .split(header_area);
        if let Some(cpu) = columns.cpu_index().map(|i| cols[i])
            && params.mouse.column >= cpu.x
            && params.mouse.column < cpu.x + cpu.width
        {
            return Some(ProcSortBy::CpuDesc);
        }
        if let Some(mem) = columns.mem_index().map(|i| cols[i])
            && params.mouse.column >= mem.x
            && params.mouse.column < mem.x + mem.width
        {
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

        if let Some(m) = params.metrics {
            let idxs = params.filtered_indices;
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

#[cfg(test)]
mod column_tests {
    use super::*;
    use ratatui::layout::{Direction, Layout, Rect};

    fn name_width(w: u16) -> u16 {
        let c = ProcColumns::for_width(w);
        let rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(c.constraints())
            .spacing(COL_SPACING)
            .split(Rect::new(0, 0, w, 1));
        rects[usize::from(c.pid)].width
    }

    /// The complaint this fixes: on a narrow pane the Name column was the first thing to
    /// disappear, leaving a table of numbers with nothing to identify the process. Name
    /// must now be the last column standing, at every width that can render anything.
    #[test]
    fn name_is_never_the_column_that_gets_dropped() {
        for width in NAME_MIN_W..=200u16 {
            assert!(
                name_width(width) >= 1,
                "width {width}: Name was squeezed to nothing"
            );
        }
    }

    /// Columns are shed in reverse priority order, so a narrower pane can never show a
    /// column that a wider one hid.
    #[test]
    fn columns_are_shed_in_priority_order() {
        for width in 0..=200u16 {
            let c = ProcColumns::for_width(width);
            assert!(!c.mem_pct || c.pid, "width {width}: Mem % outlived PID");
            assert!(!c.pid || c.mem, "width {width}: PID outlived Mem");
            assert!(!c.mem || c.cpu, "width {width}: Mem outlived CPU %");
        }
    }

    /// Columns come back as the pane widens and never flap.
    #[test]
    fn columns_are_monotonic_in_width() {
        let mut prev = ProcColumns::for_width(0);
        for width in 1..=200u16 {
            let c = ProcColumns::for_width(width);
            for (was, now, name) in [
                (prev.cpu, c.cpu, "CPU %"),
                (prev.mem, c.mem, "Mem"),
                (prev.pid, c.pid, "PID"),
                (prev.mem_pct, c.mem_pct, "Mem %"),
            ] {
                assert!(!was || now, "width {width}: {name} vanished as it widened");
            }
            prev = c;
        }
    }

    /// The tiers, from a comfortable pane down to a very narrow one.
    #[test]
    fn narrow_panes_shed_columns_in_order() {
        let full = ProcColumns::for_width(48);
        assert_eq!(full.constraints().len(), 5);
        assert!(full.pid && full.cpu && full.mem && full.mem_pct);

        // Mem % goes first.
        let c = ProcColumns::for_width(45);
        assert!(c.pid && c.mem && !c.mem_pct);

        // Then PID.
        let c = ProcColumns::for_width(35);
        assert!(!c.pid && c.cpu && c.mem);

        // Then Mem, leaving the name and its CPU load.
        let c = ProcColumns::for_width(20);
        assert!(!c.mem && c.cpu);
        assert_eq!(c.constraints().len(), 2);

        // At the floor, just the name.
        let c = ProcColumns::for_width(10);
        assert!(!c.cpu && !c.mem);
        assert_eq!(c.constraints().len(), 1);
    }

    /// Regression guard for the old behaviour: a 130-column terminal gives the process
    /// pane ~48 columns, and every column still fits there.
    #[test]
    fn a_wide_terminal_keeps_the_full_table() {
        assert_eq!(ProcColumns::for_width(48).constraints().len(), 5);
    }

    /// Sort clicks are resolved by index, so those indices must track the columns that
    /// are actually rendered — otherwise clicking "CPU %" would sort by Mem.
    #[test]
    fn sort_indices_follow_the_rendered_columns() {
        let wide = ProcColumns::for_width(48);
        assert_eq!(wide.cpu_index(), Some(2)); // PID, Name, CPU %
        assert_eq!(wide.mem_index(), Some(3));

        let narrow = ProcColumns::for_width(35);
        assert_eq!(narrow.cpu_index(), Some(1)); // Name, CPU %
        assert_eq!(narrow.mem_index(), Some(2));

        // A column that is not rendered has no index to click.
        let tiny = ProcColumns::for_width(10);
        assert_eq!(tiny.cpu_index(), None);
        assert_eq!(tiny.mem_index(), None);

        // Whatever the width, any index returned is inside the rendered set.
        for width in 0..=200u16 {
            let c = ProcColumns::for_width(width);
            let n = c.constraints().len();
            for i in [c.cpu_index(), c.mem_index()].into_iter().flatten() {
                assert!(i < n, "width {width}: index {i} outside {n} columns");
            }
        }
    }

    /// Name takes the slack, so it grows with the pane instead of being pinned to a
    /// percentage that the fixed columns can crush.
    #[test]
    fn name_absorbs_the_leftover_width() {
        assert!(
            name_width(80) > name_width(60),
            "Name did not grow with the pane"
        );
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use socktop_connector::{Metrics, ProcessInfo};

    fn metrics() -> Metrics {
        Metrics {
            sampled_at_ms: None,
            cpu_total: 0.0,
            cpu_per_core: vec![],
            mem_total: 32_000_000_000,
            mem_used: 0,
            swap_total: 0,
            swap_used: 0,
            hostname: "t".into(),
            cpu_temp_c: None,
            disks: vec![],
            networks: vec![],
            top_processes: vec![ProcessInfo {
                pid: 4242,
                name: "some-process".into(),
                cpu_usage: 1.5,
                mem_bytes: 1_000_000,
            }],
            gpus: None,
            process_count: Some(1),
        }
    }

    /// Renders the pane and returns its header row as text.
    fn header_row(width: u16) -> String {
        let m = metrics();
        let mut cache = Vec::new();
        let peak = rebuild_row_cache(&m, &mut cache);
        let idxs = [0usize];
        let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
        terminal
            .draw(|f| {
                draw_top_processes(
                    f,
                    Rect::new(0, 0, width, 8),
                    ProcessDisplayParams {
                        metrics: Some(&m),
                        scroll_offset: 0,
                        sort_by: ProcSortBy::CpuDesc,
                        selected_process_pid: None,
                        selected_process_index: None,
                        search_query: "",
                        search_active: false,
                        filtered_indices: &idxs,
                        cached_rows: &cache,
                        peak_cpu: peak,
                    },
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..width)
            .map(|x| buf[(x, 1)].symbol().to_string())
            .collect()
    }

    fn click(width: u16, column: u16) -> Option<ProcSortBy> {
        let m = metrics();
        let mut scroll = 0usize;
        let mut drag = None;
        let mut sel_pid = None;
        let mut sel_idx = None;
        let idxs = [0usize];
        processes_handle_mouse_with_selection(ProcessMouseParams {
            scroll_offset: &mut scroll,
            selected_process_pid: &mut sel_pid,
            selected_process_index: &mut sel_idx,
            drag: &mut drag,
            mouse: MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            area: Rect::new(0, 0, width, 8),
            total_rows: 1,
            metrics: Some(&m),
            search_box_visible: false,
            filtered_indices: &idxs,
        })
    }

    /// The hit-test rects are computed by a separate `Layout` call from the one `Table`
    /// renders with. This walks the rendered header text and clicks each label where it
    /// actually appears, which catches any drift between the two — including column
    /// spacing, which the two APIs configure differently.
    #[test]
    fn clicking_a_rendered_sort_header_sorts_by_that_column() {
        for width in [40u16, 50, 60, 80, 120] {
            let row = header_row(width);
            let cpu_at = row.find("CPU").map(|i| row[..i].chars().count() as u16);
            let mem_at = row.find("Mem").map(|i| row[..i].chars().count() as u16);

            if let Some(x) = cpu_at {
                assert_eq!(
                    click(width, x),
                    Some(ProcSortBy::CpuDesc),
                    "width {width}: clicking the rendered 'CPU %' header at column {x} \
                     did not sort by CPU (header row: {row:?})"
                );
            }
            if let Some(x) = mem_at {
                assert_eq!(
                    click(width, x),
                    Some(ProcSortBy::MemDesc),
                    "width {width}: clicking the rendered 'Mem' header at column {x} \
                     did not sort by Mem (header row: {row:?})"
                );
            }
        }
    }

    /// Name is what identifies the row, so it must be rendered at every width the pane
    /// can draw anything at.
    #[test]
    fn the_name_column_is_rendered_even_when_narrow() {
        for width in [30u16, 40, 60, 120] {
            let row = header_row(width);
            assert!(
                row.contains("Name"),
                "width {width}: no Name column in header {row:?}"
            );
        }
    }
}
