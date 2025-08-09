//! App state and main loop: input handling, fetching metrics, updating history, and drawing.

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction},
    Terminal,
};
use tokio::time::sleep;

use crate::history::{push_capped, PerCoreHistory};
use crate::types::Metrics;
use crate::ui::{
    cpu::{draw_cpu_avg_graph, draw_per_core_bars},
    disks::draw_disks,
    header::draw_header,
    mem::draw_mem,
    net::draw_net_spark,
    processes::draw_top_processes,
    swap::draw_swap,
};
use crate::ws::{connect, request_metrics};

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
        }
    }

    pub async fn run(&mut self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Connect to agent
        let mut ws = connect(url).await?;

        // Terminal setup
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Main loop
        let res = self.event_loop(&mut terminal, &mut ws).await;

        // Teardown
        disable_raw_mode()?;
        let backend = terminal.backend_mut();
        execute!(backend, LeaveAlternateScreen)?;
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
                if let Event::Key(k) = event::read()? {
                    if matches!(
                        k.code,
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc
                    ) {
                        self.should_quit = true;
                    }
                }
            }
            if self.should_quit {
                break;
            }

            // Fetch and update
            if let Some(m) = request_metrics(ws).await {
                self.update_with_metrics(m);
            }

            // Draw
            terminal.draw(|f| self.draw(f))?;

            // Tick rate
            sleep(Duration::from_millis(500)).await;
        }

        Ok(())
    }

    fn update_with_metrics(&mut self, m: Metrics) {
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

        self.last_metrics = Some(m);
    }

    fn draw(&mut self, f: &mut ratatui::Frame<'_>) {
        let area = f.area();

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

        draw_header(f, rows[0], self.last_metrics.as_ref());

        let top = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(rows[1]);

        draw_cpu_avg_graph(f, top[0], &self.cpu_hist, self.last_metrics.as_ref());
        draw_per_core_bars(f, top[1], self.last_metrics.as_ref(), &self.per_core_hist);

        draw_mem(f, rows[2], self.last_metrics.as_ref());
        draw_swap(f, rows[3], self.last_metrics.as_ref());

        let bottom = ratatui::layout::Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
            .split(rows[4]);

        let left_stack = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(6),
                Constraint::Length(4),
                Constraint::Length(4),
            ])
            .split(bottom[0]);

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

        draw_top_processes(f, bottom[1], self.last_metrics.as_ref());
    }
}
