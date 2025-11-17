//! Process details modal for socktop TUI application

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Axis, Block, Borders, Chart, Dataset, GraphType, Padding, Paragraph, Row, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Table,
    },
};

use super::modal::ModalManager;
use super::modal_format::{calculate_dynamic_y_max, format_uptime, normalize_cpu_usage};
use super::modal_types::{ProcessModalData, ScatterPlotParams};
use super::theme::{MODAL_BG, MODAL_HINT_FG, PROCESS_DETAILS_ACCENT};

/// Parameters for rendering memory and I/O graphs
struct MemoryIoParams<'a> {
    process: &'a socktop_connector::DetailedProcessInfo,
    mem_history: &'a std::collections::VecDeque<u64>,
    io_read_history: &'a std::collections::VecDeque<u64>,
    io_write_history: &'a std::collections::VecDeque<u64>,
    max_mem_bytes: u64,
}

impl ModalManager {
    pub(super) fn render_process_details(
        &mut self,
        f: &mut Frame,
        area: Rect,
        pid: u32,
        data: ProcessModalData,
    ) {
        let title = format!("Process Details - PID {pid}");

        // Use neutral colors to match main UI aesthetic
        let block = Block::default().title(title).borders(Borders::ALL);

        // Split the modal into the 3-row layout as designed
        let inner = block.inner(area);
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(18), // Top row: CPU sparkline | Thread scatter plot
                Constraint::Length(25), // Middle row: Memory/IO graphs | Thread table | Command details (fixed height for consistent scrolling)
                Constraint::Min(6),     // Bottom row: Journal events (gets remaining space)
                Constraint::Length(1),  // Help line
            ])
            .split(inner);

        // Render the border
        f.render_widget(block, area);

        if let Some(details) = data.details {
            // Top Row: CPU sparkline (left) | Thread scatter plot (right)
            self.render_top_row_with_sparkline(
                f,
                main_chunks[0],
                &details.process,
                data.history.cpu,
            );

            // Middle Row: Memory/IO + Thread Table + Command Details (with process metadata)
            self.render_middle_row_with_metadata(
                f,
                main_chunks[1],
                MemoryIoParams {
                    process: &details.process,
                    mem_history: data.history.mem,
                    io_read_history: data.history.io_read,
                    io_write_history: data.history.io_write,
                    max_mem_bytes: data.max_mem_bytes,
                },
            );

            // Bottom Row: Journal Events
            if let Some(journal) = data.journal {
                self.render_journal_events(f, main_chunks[2], journal);
            } else {
                self.render_loading_journal_events(f, main_chunks[2]);
            }
        } else if data.unsupported {
            // Agent doesn't support this feature
            self.render_unsupported_message(f, main_chunks[0]);
            self.render_loading_middle_row(f, main_chunks[1]);
            self.render_loading_journal_events(f, main_chunks[2]);
        } else {
            // Loading states for all sections
            self.render_loading_top_row(f, main_chunks[0]);
            self.render_loading_middle_row(f, main_chunks[1]);
            self.render_loading_journal_events(f, main_chunks[2]);
        }

        // Help line
        let help_text = vec![Line::from(vec![
            Span::styled(
                "X ",
                Style::default()
                    .fg(PROCESS_DETAILS_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("close  ", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(
                "P ",
                Style::default()
                    .fg(PROCESS_DETAILS_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "goto parent  ",
                Style::default().add_modifier(Modifier::DIM),
            ),
            Span::styled(
                "j/k ",
                Style::default()
                    .fg(PROCESS_DETAILS_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("threads  ", Style::default().add_modifier(Modifier::DIM)),
            Span::styled(
                "[ ] ",
                Style::default()
                    .fg(PROCESS_DETAILS_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("journal", Style::default().add_modifier(Modifier::DIM)),
        ])];

        let help = Paragraph::new(Text::from(help_text))
            .alignment(Alignment::Center)
            .style(Style::default());
        f.render_widget(help, main_chunks[3]);
    }
    fn render_thread_scatter_plot(
        &self,
        f: &mut Frame,
        area: Rect,
        process: &socktop_connector::DetailedProcessInfo,
    ) {
        let plot_block = Block::default()
            .title("Thread & Process CPU Time")
            .borders(Borders::ALL);

        let inner = plot_block.inner(area);

        // Convert CPU times from microseconds to milliseconds for better readability
        let main_user_ms = process.cpu_time_user as f64 / 1000.0;
        let main_system_ms = process.cpu_time_system as f64 / 1000.0;

        // Calculate max values for scaling
        let mut max_user = main_user_ms;
        let mut max_system = main_system_ms;

        for child in &process.child_processes {
            let child_user_ms = child.cpu_time_user as f64 / 1000.0;
            let child_system_ms = child.cpu_time_system as f64 / 1000.0;
            max_user = max_user.max(child_user_ms);
            max_system = max_system.max(child_system_ms);
        }

        // Add some padding to the scale
        max_user = (max_user * 1.1).max(1.0);
        max_system = (max_system * 1.1).max(1.0);

        // Render the existing scatter plot but in the smaller space
        self.render_scatter_plot_content(
            f,
            inner,
            ScatterPlotParams {
                process,
                main_user_ms,
                main_system_ms,
                max_user,
                max_system,
            },
        );

        // Render the border
        f.render_widget(plot_block, area);
    }

    fn render_memory_io_graphs(&self, f: &mut Frame, area: Rect, params: MemoryIoParams) {
        let graphs_block = Block::default()
            .title("Memory & I/O")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1));

        let mem_mb = params.process.mem_bytes as f64 / 1_048_576.0;
        let virtual_mb = params.process.virtual_mem_bytes as f64 / 1_048_576.0;

        let mut content_lines = vec![
            Line::from(vec![
                Span::styled("🧠 Memory", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(""), // Small padding
            ]),
            Line::from(vec![
                Span::styled("  RSS: ", Style::default().add_modifier(Modifier::DIM)),
                Span::raw(format!("{mem_mb:.1} MB")),
            ]),
        ];

        // Add memory sparkline if we have history
        if params.mem_history.len() >= 2 {
            let mem_data: Vec<u64> = params
                .mem_history
                .iter()
                .map(|&bytes| bytes / 1_048_576)
                .collect(); // Convert to MB
            let max_mem = mem_data.iter().copied().max().unwrap_or(1).max(1);

            // Create mini sparkline using Unicode blocks
            let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let sparkline_str: String = mem_data
                .iter()
                .map(|&val| {
                    let level = ((val as f64 / max_mem as f64) * 7.0).round() as usize;
                    blocks[level.min(7)]
                })
                .collect();

            content_lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(sparkline_str, Style::default().fg(Color::Blue)),
            ]));
        } else {
            content_lines.push(Line::from(vec![Span::styled(
                "  Collecting...",
                Style::default().add_modifier(Modifier::DIM),
            )]));
        }

        content_lines.push(Line::from(vec![
            Span::styled("  Virtual: ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(format!("{virtual_mb:.1} MB")),
        ]));

        // Add max memory if we have tracked it
        if params.max_mem_bytes > 0 {
            let max_mb = params.max_mem_bytes as f64 / 1_048_576.0;
            content_lines.push(Line::from(vec![
                Span::styled(
                    "  Max Memory: ",
                    Style::default().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!("{max_mb:.1} MB"),
                    Style::default().fg(Color::Yellow),
                ),
            ]));
        }

        // Add shared memory if available
        if let Some(shared_bytes) = params.process.shared_mem_bytes {
            let shared_mb = shared_bytes as f64 / 1_048_576.0;
            content_lines.push(Line::from(vec![
                Span::styled("  Shared: ", Style::default().add_modifier(Modifier::DIM)),
                Span::raw(format!("{shared_mb:.1} MB")),
            ]));
        }

        content_lines.push(Line::from(""));
        content_lines.push(Line::from(vec![
            Span::styled("💾 Disk I/O", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(""), // Small padding
        ]));

        // Add I/O stats if available
        match (params.process.read_bytes, params.process.write_bytes) {
            (Some(read), Some(write)) => {
                let read_mb = read as f64 / 1_048_576.0;
                let write_mb = write as f64 / 1_048_576.0;
                content_lines.push(Line::from(vec![
                    Span::styled("  Read: ", Style::default().add_modifier(Modifier::DIM)),
                    Span::raw(format!("{read_mb:.1} MB")),
                ]));

                // Add read I/O sparkline if we have history
                if params.io_read_history.len() >= 2 {
                    let read_data: Vec<u64> = params
                        .io_read_history
                        .iter()
                        .map(|&bytes| bytes / 1_048_576)
                        .collect(); // Convert to MB
                    let max_read = read_data.iter().copied().max().unwrap_or(1).max(1);

                    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                    let sparkline_str: String = read_data
                        .iter()
                        .map(|&val| {
                            let level = ((val as f64 / max_read as f64) * 7.0).round() as usize;
                            blocks[level.min(7)]
                        })
                        .collect();

                    content_lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(sparkline_str, Style::default().fg(Color::Green)),
                    ]));
                }

                content_lines.push(Line::from(vec![
                    Span::styled("  Write: ", Style::default().add_modifier(Modifier::DIM)),
                    Span::raw(format!("{write_mb:.1} MB")),
                ]));

                // Add write I/O sparkline if we have history
                if params.io_write_history.len() >= 2 {
                    let write_data: Vec<u64> = params
                        .io_write_history
                        .iter()
                        .map(|&bytes| bytes / 1_048_576)
                        .collect(); // Convert to MB
                    let max_write = write_data.iter().copied().max().unwrap_or(1).max(1);

                    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                    let sparkline_str: String = write_data
                        .iter()
                        .map(|&val| {
                            let level = ((val as f64 / max_write as f64) * 7.0).round() as usize;
                            blocks[level.min(7)]
                        })
                        .collect();

                    content_lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(sparkline_str, Style::default().fg(Color::Yellow)),
                    ]));
                }
            }
            _ => {
                content_lines.push(Line::from(vec![Span::styled(
                    "   Not available",
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            }
        }

        let content = Paragraph::new(content_lines).block(graphs_block);

        f.render_widget(content, area);
    }

    fn render_thread_table(
        &mut self,
        f: &mut Frame,
        area: Rect,
        process: &socktop_connector::DetailedProcessInfo,
    ) {
        let total_items = process.threads.len() + process.child_processes.len();

        // Manually calculate inner area (like processes.rs does)
        let inner_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };

        // Calculate visible rows: inner height minus header (1 line) and header bottom margin (1 line)
        let visible_rows = inner_area.height.saturating_sub(2).max(1) as usize;

        // Calculate and store max scroll for key handler bounds checking
        self.thread_scroll_max = if total_items > visible_rows {
            total_items.saturating_sub(visible_rows)
        } else {
            0
        };

        // Clamp scroll offset to valid range
        let scroll_offset = self.thread_scroll_offset.min(self.thread_scroll_max);

        // Combine threads and processes into rows
        let mut rows = Vec::new();

        // Add threads first
        for thread in &process.threads {
            rows.push(Row::new(vec![
                Line::from(Span::styled("[T]", Style::default().fg(Color::Cyan))),
                Line::from(format!("{}", thread.tid)),
                Line::from(thread.name.clone()),
                Line::from(thread.status.clone()),
            ]));
        }

        // Add child processes
        for child in &process.child_processes {
            rows.push(Row::new(vec![
                Line::from(Span::styled("[P]", Style::default().fg(Color::Green))),
                Line::from(format!("{}", child.pid)),
                Line::from(child.name.clone()),
                Line::from(format!("{:.1}%", child.cpu_usage)),
            ]));
        }

        // Create table header
        let header = Row::new(vec!["Type", "TID/PID", "Name", "Status/CPU"])
            .style(Style::default().add_modifier(Modifier::BOLD))
            .bottom_margin(1);

        let block = Block::default()
            .title(format!(
                "Threads ({}) & Children ({}) - j/k to scroll, u/d for 10x",
                process.threads.len(),
                process.child_processes.len()
            ))
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1));

        let table = Table::new(
            rows.iter().skip(scroll_offset).take(visible_rows).cloned(),
            [
                Constraint::Length(6),
                Constraint::Length(10),
                Constraint::Min(15),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(block)
        .highlight_style(Style::default());

        f.render_widget(table, area);

        // Render scrollbar if there are more items than visible
        if total_items > visible_rows {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            // Use the same max_scroll value we use for clamping
            // This ensures the scrollbar position matches our actual scroll range
            let mut scrollbar_state =
                ScrollbarState::new(self.thread_scroll_max).position(scroll_offset);

            let scrollbar_area = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y + 1,
                width: 1,
                height: area.height.saturating_sub(2),
            };

            f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    fn render_journal_events(
        &mut self,
        f: &mut Frame,
        area: Rect,
        journal: &socktop_connector::JournalResponse,
    ) {
        let total_entries = journal.entries.len();
        let visible_lines = area.height.saturating_sub(2) as usize; // Account for borders

        // Calculate and store max scroll for key handler bounds checking
        self.journal_scroll_max = if total_entries > visible_lines {
            total_entries.saturating_sub(visible_lines)
        } else {
            0
        };

        // Clamp scroll offset to valid range
        let scroll_offset = self.journal_scroll_offset.min(self.journal_scroll_max);

        let journal_block = Block::default()
            .title(format!(
                "Journal Events ({total_entries} entries) - Use [ ] to scroll"
            ))
            .borders(Borders::ALL);

        let content_lines: Vec<Line> = if journal.entries.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No journal entries found for this process",
                    Style::default().add_modifier(Modifier::DIM),
                )),
            ]
        } else {
            journal
                .entries
                .iter()
                .skip(scroll_offset)
                .take(visible_lines)
                .map(|entry| {
                    let priority_style = match entry.priority {
                        socktop_connector::LogLevel::Error
                        | socktop_connector::LogLevel::Critical => Style::default().fg(Color::Red),
                        socktop_connector::LogLevel::Warning => Style::default().fg(Color::Yellow),
                        socktop_connector::LogLevel::Info | socktop_connector::LogLevel::Notice => {
                            Style::default().fg(Color::Blue)
                        }
                        _ => Style::default(),
                    };

                    let timestamp = &entry.timestamp[..entry.timestamp.len().min(16)]; // Show just time
                    let message_max_len = area.width.saturating_sub(30) as usize; // Leave space for timestamp + priority
                    let message = &entry.message[..entry.message.len().min(message_max_len)];

                    Line::from(vec![
                        Span::styled(timestamp, Style::default().add_modifier(Modifier::DIM)),
                        Span::raw(" "),
                        Span::styled(
                            format!("{:>7}", format!("{:?}", entry.priority)),
                            priority_style,
                        ),
                        Span::raw(" "),
                        Span::raw(message),
                        if entry.message.len() > message_max_len {
                            Span::styled("...", Style::default().add_modifier(Modifier::DIM))
                        } else {
                            Span::raw("")
                        },
                    ])
                })
                .collect()
        };

        let content = Paragraph::new(content_lines).block(journal_block);

        f.render_widget(content, area);

        // Render scrollbar if there are more entries than visible
        if total_entries > visible_lines {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓"));

            // Use the same max_scroll value we use for clamping
            let mut scrollbar_state =
                ScrollbarState::new(self.journal_scroll_max).position(scroll_offset);

            let scrollbar_area = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y + 1,
                width: 1,
                height: area.height.saturating_sub(2),
            };

            f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
        }
    }

    fn render_scatter_plot_content(&self, f: &mut Frame, area: Rect, params: ScatterPlotParams) {
        if area.width < 20 || area.height < 10 {
            // Area too small for meaningful plot
            let content = Paragraph::new(vec![Line::from(Span::styled(
                "Area too small for plot",
                Style::default().fg(MODAL_HINT_FG),
            ))])
            .alignment(Alignment::Center)
            .style(Style::default().bg(MODAL_BG));
            f.render_widget(content, area);
            return;
        }

        // Calculate plot dimensions (leave space for axes labels + legend)
        let plot_width = area.width.saturating_sub(8) as usize; // Leave space for Y-axis labels
        let plot_height = area.height.saturating_sub(6) as usize; // Leave space for legend (3 lines) + X-axis labels (2 lines) + title (1 line)

        if plot_width == 0 || plot_height == 0 {
            return;
        }

        // Create a 2D grid to represent the plot
        let mut plot_grid = vec![vec![' '; plot_width]; plot_height];

        // Plot main process
        let main_x = ((params.main_user_ms / params.max_user) * (plot_width - 1) as f64) as usize;
        let main_y = plot_height.saturating_sub(1).saturating_sub(
            ((params.main_system_ms / params.max_system) * (plot_height - 1) as f64) as usize,
        );
        if main_x < plot_width && main_y < plot_height {
            plot_grid[main_y][main_x] = '●'; // Main process marker
        }

        // Plot threads (use different marker)
        for thread in &params.process.threads {
            let thread_user_ms = thread.cpu_time_user as f64 / 1000.0;
            let thread_system_ms = thread.cpu_time_system as f64 / 1000.0;

            let thread_x = ((thread_user_ms / params.max_user) * (plot_width - 1) as f64) as usize;
            let thread_y = plot_height.saturating_sub(1).saturating_sub(
                ((thread_system_ms / params.max_system) * (plot_height - 1) as f64) as usize,
            );

            if thread_x < plot_width && thread_y < plot_height {
                if plot_grid[thread_y][thread_x] == ' ' {
                    plot_grid[thread_y][thread_x] = '○'; // Thread marker (hollow circle)
                } else if plot_grid[thread_y][thread_x] == '○' {
                    plot_grid[thread_y][thread_x] = '◎'; // Multiple threads at same point
                } else {
                    plot_grid[thread_y][thread_x] = '◉'; // Mixed threads/processes at same point
                }
            }
        }

        // Plot child processes
        for child in &params.process.child_processes {
            let child_user_ms = child.cpu_time_user as f64 / 1000.0;
            let child_system_ms = child.cpu_time_system as f64 / 1000.0;

            let child_x = ((child_user_ms / params.max_user) * (plot_width - 1) as f64) as usize;
            let child_y = plot_height.saturating_sub(1).saturating_sub(
                ((child_system_ms / params.max_system) * (plot_height - 1) as f64) as usize,
            );

            if child_x < plot_width && child_y < plot_height {
                if plot_grid[child_y][child_x] == ' ' {
                    plot_grid[child_y][child_x] = '•'; // Child process marker
                } else {
                    plot_grid[child_y][child_x] = '◉'; // Multiple items at same point
                }
            }
        }

        // Render the plot
        let mut lines = Vec::new();

        // Add Y-axis labels and plot content
        for (i, row) in plot_grid.iter().enumerate() {
            let y_value = params.max_system * (1.0 - (i as f64 / (plot_height - 1) as f64));
            // Always format with 4 characters width, right-aligned, to prevent axis shifting
            let y_label = if y_value >= 100.0 {
                format!("{y_value:>4.0}")
            } else {
                format!("{y_value:>4.1}")
            };

            let plot_content: String = row.iter().collect();

            lines.push(Line::from(vec![
                Span::styled(y_label, Style::default()),
                Span::styled(" │", Style::default()),
                Span::styled(plot_content, Style::default()),
            ]));
        }

        // Add X-axis
        let x_axis_padding = "     ".to_string(); // Match Y-axis label width
        let x_axis_line = "─".repeat(plot_width + 1);
        lines.push(Line::from(vec![
            Span::styled(x_axis_padding, Style::default()),
            Span::styled(x_axis_line, Style::default()),
        ]));

        // Add X-axis labels
        let x_label_start = "0.0".to_string();
        let x_label_mid = format!("{:.1}", params.max_user / 2.0);
        let x_label_end = format!("{:.1}", params.max_user);

        let spacing = plot_width / 3;
        let x_labels = format!(
            "     {}{}{}{}{}",
            x_label_start,
            " ".repeat(spacing.saturating_sub(x_label_start.len())),
            x_label_mid,
            " ".repeat(spacing.saturating_sub(x_label_mid.len())),
            x_label_end
        );

        lines.push(Line::from(vec![Span::styled(x_labels, Style::default())]));

        // Add axis titles with better visibility
        lines.push(Line::from(vec![Span::styled(
            "     User CPU Time (ms) →",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));

        // Add Y-axis label and legend at the top
        lines.insert(
            0,
            Line::from(vec![Span::styled(
                "↑ System CPU Time (ms)",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )]),
        );
        lines.insert(
            1,
            Line::from(vec![Span::styled(
                "● Main  ○ Thread  • Child  ◉ Multiple",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            )]),
        );
        lines.insert(2, Line::from("")); // Spacing after legend

        let content = Paragraph::new(lines)
            .style(Style::default())
            .alignment(Alignment::Left);

        f.render_widget(content, area);
    }

    fn render_loading_top_row(&self, f: &mut Frame, area: Rect) {
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        self.render_loading_metadata(f, top_chunks[0]);
        self.render_loading_scatter(f, top_chunks[1]);
    }

    fn render_loading_middle_row(&self, f: &mut Frame, area: Rect) {
        let middle_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ])
            .split(area);

        self.render_loading_graphs(f, middle_chunks[0]);
        self.render_loading_table(f, middle_chunks[1]);
        self.render_loading_command(f, middle_chunks[2]);
    }

    fn render_loading_metadata(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Process Info & CPU History")
            .borders(Borders::ALL);

        let content = Paragraph::new("Loading process metadata...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_loading_scatter(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Thread CPU Time Distribution")
            .borders(Borders::ALL);

        let content = Paragraph::new("Loading CPU time data...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_loading_graphs(&self, f: &mut Frame, area: Rect) {
        let block = Block::default().title("Memory & I/O").borders(Borders::ALL);

        let content = Paragraph::new("Loading memory & I/O data...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_loading_table(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Child Processes")
            .borders(Borders::ALL);

        let content = Paragraph::new("Loading child process data...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_loading_command(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Command & Details")
            .borders(Borders::ALL);

        let content = Paragraph::new("Loading command details...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_loading_journal_events(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Journal Events")
            .borders(Borders::ALL);

        let content = Paragraph::new("Loading journal entries...")
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().add_modifier(Modifier::DIM));

        f.render_widget(content, area);
    }

    fn render_unsupported_message(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title("Process Details")
            .borders(Borders::ALL);

        let content = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "⚠  Agent Update Required",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "This agent version does not support per-process metrics.",
                Style::default().add_modifier(Modifier::DIM),
            )),
            Line::from(Span::styled(
                "Please update your socktop_agent to the latest version.",
                Style::default().add_modifier(Modifier::DIM),
            )),
        ])
        .block(block)
        .alignment(Alignment::Center);

        f.render_widget(content, area);
    }

    fn render_top_row_with_sparkline(
        &self,
        f: &mut Frame,
        area: Rect,
        process: &socktop_connector::DetailedProcessInfo,
        cpu_history: &std::collections::VecDeque<f32>,
    ) {
        // Split top row: CPU sparkline (left 60%) | Thread scatter plot (right 40%)
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60), // CPU sparkline
                Constraint::Percentage(40), // Thread scatter plot
            ])
            .split(area);

        self.render_cpu_sparkline(f, top_chunks[0], process, cpu_history);
        self.render_thread_scatter_plot(f, top_chunks[1], process);
    }

    fn render_cpu_sparkline(
        &self,
        f: &mut Frame,
        area: Rect,
        process: &socktop_connector::DetailedProcessInfo,
        cpu_history: &std::collections::VecDeque<f32>,
    ) {
        // Normalize CPU to 0-100% by dividing by thread count
        // This shows per-core utilization rather than total utilization across all cores
        let thread_count = process.thread_count;

        // Calculate actual average and current (normalized to 0-100%)
        let current_cpu =
            normalize_cpu_usage(cpu_history.back().copied().unwrap_or(0.0), thread_count);
        let avg_cpu = if cpu_history.is_empty() {
            0.0
        } else {
            let total: f32 = cpu_history.iter().sum();
            normalize_cpu_usage(total / cpu_history.len() as f32, thread_count)
        };
        let title = format!("CPU (now: {current_cpu:.1}% | {avg_cpu:.1}%)");

        // Similar to main CPU rendering but for process CPU
        if cpu_history.len() < 2 {
            let block = Block::default().title(title).borders(Borders::ALL);
            let inner = block.inner(area);
            f.render_widget(block, area);

            let content = Paragraph::new("Collecting CPU history data...")
                .alignment(Alignment::Center)
                .style(Style::default().add_modifier(Modifier::DIM));
            f.render_widget(content, inner);
            return;
        }

        let max_points = area.width.saturating_sub(10) as usize; // Leave room for Y-axis labels
        let start = cpu_history.len().saturating_sub(max_points);

        // Create data points for the chart (normalized to 0-100%)
        let data: Vec<(f64, f64)> = cpu_history
            .iter()
            .skip(start)
            .enumerate()
            .map(|(i, &val)| {
                let normalized = normalize_cpu_usage(val, thread_count);
                (i as f64, normalized as f64)
            })
            .collect();

        let datasets = vec![
            Dataset::default()
                .name("CPU %")
                .marker(ratatui::symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&data),
        ];

        let x_max = data.len().max(1) as f64;

        // Dynamic Y-axis scaling in 10% increments
        let max_cpu = data.iter().map(|(_, y)| *y).fold(0.0f64, f64::max);
        let y_max = calculate_dynamic_y_max(max_cpu);

        let y_labels = vec![
            Line::from("0%"),
            Line::from(format!("{:.0}%", y_max / 2.0)),
            Line::from(format!("{y_max:.0}%")),
        ];

        let chart = Chart::new(datasets)
            .block(Block::default().borders(Borders::ALL).title(title))
            .x_axis(
                Axis::default()
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0.0, x_max]),
            )
            .y_axis(
                Axis::default()
                    .style(Style::default().fg(Color::Gray))
                    .labels(y_labels)
                    .bounds([0.0, y_max]),
            );

        f.render_widget(chart, area);
    }

    fn render_middle_row_with_metadata(
        &mut self,
        f: &mut Frame,
        area: Rect,
        params: MemoryIoParams,
    ) {
        // Split middle row: Memory/IO (30%) | Thread table (40%) | Command + Metadata (30%)
        let middle_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(40),
                Constraint::Percentage(30),
            ])
            .split(area);

        self.render_memory_io_graphs(
            f,
            middle_chunks[0],
            MemoryIoParams {
                process: params.process,
                mem_history: params.mem_history,
                io_read_history: params.io_read_history,
                io_write_history: params.io_write_history,
                max_mem_bytes: params.max_mem_bytes,
            },
        );
        self.render_thread_table(f, middle_chunks[1], params.process);
        self.render_command_and_metadata(f, middle_chunks[2], params.process);
    }

    fn render_command_and_metadata(
        &self,
        f: &mut Frame,
        area: Rect,
        process: &socktop_connector::DetailedProcessInfo,
    ) {
        let details_block = Block::default()
            .title("Command & Details")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1));

        // Calculate uptime
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let uptime_secs = now.saturating_sub(process.start_time);
        let uptime_str = format_uptime(uptime_secs);

        // Format CPU times
        let user_time_sec = process.cpu_time_user as f64 / 1_000_000.0;
        let system_time_sec = process.cpu_time_system as f64 / 1_000_000.0;

        let mut content_lines = vec![
            Line::from(vec![
                Span::styled(
                    "⚡ Status:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(&process.status),
            ]),
            Line::from(vec![
                Span::styled(
                    "⏱️ Uptime:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(uptime_str),
            ]),
            Line::from(vec![
                Span::styled(
                    "🧵 Threads:  ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}", process.thread_count)),
            ]),
            Line::from(vec![
                Span::styled(
                    "👶 Children: ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{}", process.child_processes.len())),
            ]),
        ];

        // Add file descriptors if available
        if let Some(fd_count) = process.fd_count {
            content_lines.push(Line::from(vec![
                Span::styled(
                    "📁 FDs:      ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{fd_count}")),
            ]));
        }

        content_lines.push(Line::from(""));

        // Process hierarchy with clickable parent PID
        if let Some(ppid) = process.parent_pid {
            content_lines.push(Line::from(vec![
                Span::styled(
                    "👪 Parent:   ",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{ppid}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    " [P]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::DIM),
                ),
            ]));
        }

        content_lines.push(Line::from(vec![
            Span::styled("👤 UID: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("{}", process.user_id)),
            Span::styled("  👥 GID: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("{}", process.group_id)),
        ]));

        content_lines.push(Line::from(""));
        content_lines.push(Line::from(vec![Span::styled(
            "⏲️ CPU Time",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        content_lines.push(Line::from(vec![
            Span::styled("   User: ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(format!("{user_time_sec:.2}s")),
        ]));
        content_lines.push(Line::from(vec![
            Span::styled("   System: ", Style::default().add_modifier(Modifier::DIM)),
            Span::raw(format!("{system_time_sec:.2}s")),
        ]));

        content_lines.push(Line::from(""));

        // Executable path if available
        if let Some(exe) = &process.executable_path {
            content_lines.push(Line::from(vec![Span::styled(
                "📂 Executable",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            // Truncate if too long
            let max_width = (area.width.saturating_sub(6)) as usize;
            if exe.len() > max_width {
                let truncated = format!("...{}", &exe[exe.len().saturating_sub(max_width - 3)..]);
                content_lines.push(Line::from(vec![Span::styled(
                    format!("  {truncated}"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            } else {
                content_lines.push(Line::from(vec![Span::styled(
                    format!("  {exe}"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            }
        }

        // Working directory if available
        if let Some(cwd) = &process.working_directory {
            content_lines.push(Line::from(""));
            content_lines.push(Line::from(vec![Span::styled(
                "📁 Working Dir",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            // Truncate if too long
            let max_width = (area.width.saturating_sub(6)) as usize;
            if cwd.len() > max_width {
                let truncated = format!("...{}", &cwd[cwd.len().saturating_sub(max_width - 3)..]);
                content_lines.push(Line::from(vec![Span::styled(
                    format!("  {truncated}"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            } else {
                content_lines.push(Line::from(vec![Span::styled(
                    format!("  {cwd}"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            }
        }

        content_lines.push(Line::from(""));

        // Add command line (wrap if needed)
        content_lines.push(Line::from(vec![Span::styled(
            "⚙️ Command",
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        // Split command into multiple lines if too long
        let cmd_text = &process.command;
        let max_width = (area.width.saturating_sub(6)) as usize; // More conservative to avoid wrapping issues
        if cmd_text.len() > max_width {
            for chunk in cmd_text.as_bytes().chunks(max_width) {
                if let Ok(s) = std::str::from_utf8(chunk) {
                    content_lines.push(Line::from(vec![Span::styled(
                        format!("  {s}"),
                        Style::default().add_modifier(Modifier::DIM),
                    )]));
                }
            }
        } else {
            content_lines.push(Line::from(vec![Span::styled(
                format!("  {cmd_text}"),
                Style::default().add_modifier(Modifier::DIM),
            )]));
        }

        let content = Paragraph::new(content_lines).block(details_block);

        f.render_widget(content, area);
    }
}
