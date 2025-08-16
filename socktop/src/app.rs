//! App state and main loop: input handling, fetching metrics, updating history, and drawing.

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Rect},
    //style::Color, // + add Color
    Terminal,
};
use tokio::time::sleep;

use crate::history::{push_capped, PerCoreHistory};
use crate::types::Metrics;
use crate::ui::cpu::{
    draw_cpu_avg_graph, draw_per_core_bars, per_core_clamp, per_core_content_area,
    per_core_handle_key, per_core_handle_mouse, per_core_handle_scrollbar_mouse, PerCoreScrollDrag,
};
use crate::ui::processes::{processes_handle_key, processes_handle_mouse, ProcSortBy};
use crate::ui::{
    disks::draw_disks, gpu::draw_gpu, header::draw_header, mem::draw_mem, net::draw_net_spark,
    swap::draw_swap,
};
use crate::ws::{connect, request_disks, request_metrics, request_processes};

pub struct App {
    // Latest metrics + histories
    last_metrics: Option<Metrics>,

    // CPU avg history (0..100)
    cpu_hist: VecDeque<u64>,

    // Per-core history (0..100)
    per_core_hist: PerCoreHistory,

    // Network totals snapshot + histories of KB/s
    last_net_totals: Option<(u64, u64, Instant)>,
    rx_hist: VecDeque<u64>,
    tx_hist: VecDeque<u64>,
    rx_peak: u64,
    tx_peak: u64,

    // Quit flag
    should_quit: bool,

    pub per_core_scroll: usize,
    pub per_core_drag: Option<PerCoreScrollDrag>, // new: drag state
    pub procs_scroll_offset: usize,
    pub procs_drag: Option<PerCoreScrollDrag>,
    pub procs_sort_by: ProcSortBy,
    last_procs_area: Option<ratatui::layout::Rect>,

    last_procs_poll: Instant,
    last_disks_poll: Instant,
    procs_interval: Duration,
    disks_interval: Duration,
}

impl App {
    pub fn new() -> Self {
        Self {
            last_metrics: None,
            cpu_hist: VecDeque::with_capacity(600),
            per_core_hist: PerCoreHistory::new(60),
            last_net_totals: None,
            rx_hist: VecDeque::with_capacity(600),
            tx_hist: VecDeque::with_capacity(600),
            rx_peak: 0,
            tx_peak: 0,
            should_quit: false,
            per_core_scroll: 0,
            per_core_drag: None,
            procs_scroll_offset: 0,
            procs_drag: None,
            procs_sort_by: ProcSortBy::CpuDesc,
            last_procs_area: None,
            last_procs_poll: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now), // trigger immediately on first loop
            last_disks_poll: Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now),
            procs_interval: Duration::from_secs(2),
            disks_interval: Duration::from_secs(5),
        }
    }

    pub async fn run(
        &mut self,
        url: &str,
        tls_ca: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Connect to agent
        let mut ws = connect(url, tls_ca).await?;

        // Terminal setup
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Main loop
        let res = self.event_loop(&mut terminal, &mut ws).await;

        // Teardown
        disable_raw_mode()?;
        let backend = terminal.backend_mut();
        execute!(backend, DisableMouseCapture, LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        res
    }

    async fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        ws: &mut crate::ws::WsStream,
    ) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            // Input (non-blocking)
            while event::poll(Duration::from_millis(10))? {
                match event::read()? {
                    Event::Key(k) => {
                        if matches!(
                            k.code,
                            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc
                        ) {
                            self.should_quit = true;
                        }
                        // Per-core scroll via keys (Up/Down/PageUp/PageDown/Home/End)
                        let sz = terminal.size()?;
                        let area = Rect::new(0, 0, sz.width, sz.height);
                        let rows = ratatui::layout::Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Ratio(1, 3),
                                Constraint::Length(3),
                                Constraint::Length(3),
                                Constraint::Min(10),
                            ])
                            .split(area);
                        let top = ratatui::layout::Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
                            .split(rows[1]);
                        let content = per_core_content_area(top[1]);

                        per_core_handle_key(&mut self.per_core_scroll, k, content.height as usize);

                        let total_rows = self
                            .last_metrics
                            .as_ref()
                            .map(|mm| mm.cpu_per_core.len())
                            .unwrap_or(0);
                        per_core_clamp(
                            &mut self.per_core_scroll,
                            total_rows,
                            content.height as usize,
                        );

                        if let Some(p_area) = self.last_procs_area {
                            // page size = visible rows (inner height minus header = 1)
                            let page = p_area.height.saturating_sub(3).max(1) as usize; // borders (2) + header (1)
                            processes_handle_key(&mut self.procs_scroll_offset, k, page);
                        }
                    }
                    Event::Mouse(m) => {
                        // Layout to get areas
                        let sz = terminal.size()?;
                        let area = Rect::new(0, 0, sz.width, sz.height);
                        let rows = ratatui::layout::Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(1),
                                Constraint::Ratio(1, 3),
                                Constraint::Length(3),
                                Constraint::Length(3),
                                Constraint::Min(10),
                            ])
                            .split(area);
                        let top = ratatui::layout::Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
                            .split(rows[1]);

                        // Content wheel scrolling
                        let content = per_core_content_area(top[1]);
                        per_core_handle_mouse(
                            &mut self.per_core_scroll,
                            m,
                            content,
                            content.height as usize,
                        );

                        // Scrollbar clicks/drag
                        let total_rows = self
                            .last_metrics
                            .as_ref()
                            .map(|mm| mm.cpu_per_core.len())
                            .unwrap_or(0);
                        per_core_handle_scrollbar_mouse(
                            &mut self.per_core_scroll,
                            &mut self.per_core_drag,
                            m,
                            top[1],
                            total_rows,
                        );

                        // Clamp to bounds
                        per_core_clamp(
                            &mut self.per_core_scroll,
                            total_rows,
                            content.height as usize,
                        );

                        // Processes table: sort by column on header click
                        if let (Some(mm), Some(p_area)) =
                            (self.last_metrics.as_ref(), self.last_procs_area)
                        {
                            if let Some(new_sort) = processes_handle_mouse(
                                &mut self.procs_scroll_offset,
                                &mut self.procs_drag,
                                m,
                                p_area,
                                mm.top_processes.len(),
                            ) {
                                self.procs_sort_by = new_sort;
                            }
                        }
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
            if self.should_quit {
                break;
            }

            // Fetch and update
            if let Some(m) = request_metrics(ws).await {
                self.update_with_metrics(m);

                // Only poll processes every 2s
                if self.last_procs_poll.elapsed() >= self.procs_interval {
                    if let Some(procs) = request_processes(ws).await {
                        if let Some(mm) = self.last_metrics.as_mut() {
                            mm.top_processes = procs.top_processes;
                            mm.process_count = Some(procs.process_count);
                        }
                    }
                    self.last_procs_poll = Instant::now();
                }

                // Only poll disks every 5s
                if self.last_disks_poll.elapsed() >= self.disks_interval {
                    if let Some(disks) = request_disks(ws).await {
                        if let Some(mm) = self.last_metrics.as_mut() {
                            mm.disks = disks;
                        }
                    }
                    self.last_disks_poll = Instant::now();
                }
            }

            // Draw
            terminal.draw(|f| self.draw(f))?;

            // Tick rate
            sleep(Duration::from_millis(500)).await;
        }

        Ok(())
    }

    fn update_with_metrics(&mut self, mut m: Metrics) {
        if let Some(prev) = &self.last_metrics {
            // Preserve slower fields when the fast payload omits them
            if m.disks.is_empty() {
                m.disks = prev.disks.clone();
            }
            if m.top_processes.is_empty() {
                m.top_processes = prev.top_processes.clone();
            }
            // Preserve total processes count across fast updates
            if m.process_count.is_none() {
                m.process_count = prev.process_count;
            }
        }

        // CPU avg history
        let v = m.cpu_total.clamp(0.0, 100.0).round() as u64;
        push_capped(&mut self.cpu_hist, v, 600);

        // Per-core history (push current samples)
        self.per_core_hist.ensure_cores(m.cpu_per_core.len());
        self.per_core_hist.push_samples(&m.cpu_per_core);

        // NET: sum across all ifaces, compute KB/s via elapsed time
        let now = Instant::now();
        let rx_total = m.networks.iter().map(|n| n.received).sum::<u64>();
        let tx_total = m.networks.iter().map(|n| n.transmitted).sum::<u64>();
        let (rx_kb, tx_kb) = if let Some((prx, ptx, pts)) = self.last_net_totals {
            let dt = now.duration_since(pts).as_secs_f64().max(1e-6);
            let rx = ((rx_total.saturating_sub(prx)) as f64 / dt / 1024.0).round() as u64;
            let tx = ((tx_total.saturating_sub(ptx)) as f64 / dt / 1024.0).round() as u64;
            (rx, tx)
        } else {
            (0, 0)
        };
        self.last_net_totals = Some((rx_total, tx_total, now));
        push_capped(&mut self.rx_hist, rx_kb, 600);
        push_capped(&mut self.tx_hist, tx_kb, 600);
        self.rx_peak = self.rx_peak.max(rx_kb);
        self.tx_peak = self.tx_peak.max(tx_kb);

        // Store merged snapshot
        self.last_metrics = Some(m);
    }

    pub fn draw(&mut self, f: &mut ratatui::Frame<'_>) {
        let area = f.area();

        // Root rows: header, top (cpu avg + per-core), memory, swap, bottom
        let rows = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),   // header
                Constraint::Ratio(1, 3), // top row
                Constraint::Length(3),   // memory (left) + GPU (right, part 1)
                Constraint::Length(3),   // swap (left)   + GPU (right, part 2)
                Constraint::Min(10),     // bottom: disks + net (left), top procs (right)
            ])
            .split(area);

        // Header
        draw_header(f, rows[0], self.last_metrics.as_ref());

        // Top row: left CPU avg, right Per-core (full top-right)
        let top_lr = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(rows[1]);

        draw_cpu_avg_graph(f, top_lr[0], &self.cpu_hist, self.last_metrics.as_ref());
        draw_per_core_bars(
            f,
            top_lr[1],
            self.last_metrics.as_ref(),
            &self.per_core_hist,
            self.per_core_scroll,
        );

        // Memory + Swap rows split into left/right columns
        let mem_lr = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(rows[2]);
        let swap_lr = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(rows[3]);

        // Left: Memory + Swap
        draw_mem(f, mem_lr[0], self.last_metrics.as_ref());
        draw_swap(f, swap_lr[0], self.last_metrics.as_ref());

        // Right: GPU spans the same vertical space as Memory + Swap
        let gpu_area = ratatui::layout::Rect {
            x: mem_lr[1].x,
            y: mem_lr[1].y,
            width: mem_lr[1].width,
            height: mem_lr[1].height + swap_lr[1].height,
        };
        draw_gpu(f, gpu_area, self.last_metrics.as_ref());

        // Bottom area: left = Disks + Network, right = Top Processes
        let bottom_lr = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(rows[4]);

        // Left bottom: Disks + Net stacked (make net panes slightly taller)
        let left_stack = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    // Disks shrink slightly
                Constraint::Length(5), // Download taller
                Constraint::Length(5), // Upload taller
            ])
            .split(bottom_lr[0]);

        draw_disks(f, left_stack[0], self.last_metrics.as_ref());
        draw_net_spark(
            f,
            left_stack[1],
            &format!(
                "Download (KB/s) — now: {} | peak: {}",
                self.rx_hist.back().copied().unwrap_or(0),
                self.rx_peak
            ),
            &self.rx_hist,
            ratatui::style::Color::Green,
        );
        draw_net_spark(
            f,
            left_stack[2],
            &format!(
                "Upload (KB/s) — now: {} | peak: {}",
                self.tx_hist.back().copied().unwrap_or(0),
                self.tx_peak
            ),
            &self.tx_hist,
            ratatui::style::Color::Blue,
        );

        // Right bottom: Top Processes fills the column
        let procs_area = bottom_lr[1];
        // Cache for input handlers
        self.last_procs_area = Some(procs_area);
        crate::ui::processes::draw_top_processes(
            f,
            procs_area,
            self.last_metrics.as_ref(),
            self.procs_scroll_offset,
            self.procs_sort_by,
        );
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            last_metrics: None,
            cpu_hist: VecDeque::with_capacity(600),
            per_core_hist: PerCoreHistory::new(60),
            last_net_totals: None,
            rx_hist: VecDeque::with_capacity(600),
            tx_hist: VecDeque::with_capacity(600),
            rx_peak: 0,
            tx_peak: 0,
            should_quit: false,
            per_core_scroll: 0,
            per_core_drag: None,
            procs_scroll_offset: 0,
            procs_drag: None,
            procs_sort_by: ProcSortBy::CpuDesc,
            last_procs_area: None,
            last_procs_poll: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now), // trigger immediately on first loop
            last_disks_poll: Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now),
            procs_interval: Duration::from_secs(2),
            disks_interval: Duration::from_secs(5),
        }
    }
}
