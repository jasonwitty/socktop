//! App state and main loop: input handling, fetching metrics, updating history, and drawing.

use std::{
    collections::VecDeque,
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    //style::Color, // + add Color
    Terminal,
    backend::CrosstermBackend,
    layout::Rect,
};
use tokio::time::{sleep, timeout};

use crate::history::{PerCoreHistory, push_capped};
use crate::retry::{RetryTiming, compute_retry_timing};
use crate::types::Metrics;
use crate::ui::cpu::{
    PerCoreScrollDrag, draw_cpu_avg_graph, draw_per_core_bars, per_core_clamp,
    per_core_content_area, per_core_handle_key, per_core_handle_mouse,
    per_core_handle_scrollbar_mouse,
};
use crate::ui::layout::{AppLayout, compute as compute_layout};
use crate::ui::modal::{ModalAction, ModalManager, ModalType};
use crate::ui::processes::{
    ProcSortBy, ProcessKeyParams, processes_handle_key_with_selection,
    processes_handle_mouse_with_selection,
};
use crate::ui::{
    disks::draw_disks,
    gpu::{draw_gpu, draw_gpu_compact},
    header::{HeaderState, build_header, draw_header},
    mem::draw_mem,
    net::draw_net_spark,
    swap::draw_swap,
};

use socktop_connector::{
    AgentRequest, AgentResponse, SocktopConnector, connect_to_socktop_agent,
    connect_to_socktop_agent_with_tls,
};

// Constants for minimum intervals to ensure reasonable performance
const MIN_METRICS_INTERVAL_MS: u64 = 100;
const MIN_PROCESSES_INTERVAL_MS: u64 = 200;

/// Budget for one request/response round trip. Replies are matched to
/// requests by order, so a request that never answers would otherwise hang
/// `ws.next()` forever and freeze the TUI (raw mode even eats Ctrl+C).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Drop duplicate-name entries from a disks payload (the agent occasionally
/// reports a partition twice). Done once when fresh disk data arrives so the
/// per-frame draw path doesn't have to rebuild a HashSet.
fn dedup_disks(disks: &mut Vec<socktop_connector::DiskInfo>) {
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(disks.len());
    disks.retain(|d| seen.insert(d.name.clone()));
}

/// Outcome of draining input: keep going, or restart the event loop because a
/// reconnect installed a replacement connection.
enum InputFlow {
    Continue,
    RestartConnection,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    Reconnecting,
}

pub struct App {
    // Latest metrics + histories
    last_metrics: Option<Metrics>,

    // CPU avg history (0..100) with a running sum so draw avoids a 600-element fold per frame
    cpu_hist: VecDeque<u64>,
    cpu_hist_sum: u64,

    // Per-core history (0..100)
    per_core_hist: PerCoreHistory,

    // Network totals snapshot + histories of KB/s
    last_net_totals: Option<(u64, u64, Instant)>,
    // Agent-side sample timestamp of the previous snapshot (1.60+ agents).
    last_net_sampled_at_ms: Option<u64>,

    // Consecutive metrics-request timeouts. One timeout gets a silent stream
    // refresh; a second in a row means the agent accepts connections but
    // never answers, and deserves a persistent error instead of an invisible
    // reconnect loop that starves the UI.
    consecutive_request_timeouts: u32,
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

    // Process selection state
    pub selected_process_pid: Option<u32>,
    pub selected_process_index: Option<usize>, // Index in the visible/sorted list
    prev_selected_process_pid: Option<u32>,    // Track previous selection to detect changes

    // Process search state
    pub process_search_active: bool,
    pub process_search_query: String,

    // Cached filtered + sorted process indices. Refreshed lazily when any of
    // (metrics, sort order, search query) changes — input handlers, the draw
    // path, and auto-scroll all read from this slice so we avoid rebuilding
    // an indices Vec on every event.
    procs_filtered: Vec<usize>,
    procs_filter_dirty: bool,
    // Pre-formatted process-row strings, rebuilt once per procs poll. Indexed
    // parallel to `last_metrics.top_processes`.
    procs_row_cache: Vec<crate::ui::processes::CachedRow>,
    procs_row_peak_cpu: f32,

    last_procs_poll: Instant,
    last_disks_poll: Instant,
    procs_interval: Duration,
    disks_interval: Duration,
    metrics_interval: Duration,

    // Process details polling
    pub process_details: Option<socktop_connector::ProcessMetricsResponse>,
    pub journal_entries: Option<socktop_connector::JournalResponse>,
    pub process_cpu_history: VecDeque<f32>, // CPU history for sparkline (last 60 samples)
    pub process_cpu_history_sum: f32,       // running sum of process_cpu_history
    pub process_mem_history: VecDeque<u64>, // Memory usage history in bytes (last 60 samples)
    pub process_io_read_history: VecDeque<u64>, // Disk read DELTA history in bytes (last 60 samples)
    pub process_io_write_history: VecDeque<u64>, // Disk write DELTA history in bytes (last 60 samples)
    last_io_read_bytes: Option<u64>,             // Previous read bytes for delta calculation
    last_io_write_bytes: Option<u64>,            // Previous write bytes for delta calculation
    pub max_process_mem_bytes: u64, // Maximum memory usage observed for current process
    pub process_details_unsupported: bool, // Track if agent doesn't support process details
    last_process_details_poll: Instant,
    last_journal_poll: Instant,
    process_details_interval: Duration,
    journal_interval: Duration,

    // For reconnects
    ws_url: String,
    tls_ca: Option<String>,
    verify_hostname: bool,
    // Security / status flags
    pub is_tls: bool,
    pub has_token: bool,

    // --compact: pin the compact layout regardless of window size. Without it the
    // layout switches on its own once the window is too short for the Disks pane.
    force_compact: bool,

    // Cached title strings — only rebuilt when source values change so the
    // diff renderer can suppress redraws on idle frames.
    header_title: String,
    header_intervals_text: String,
    header_key: (String, bool, bool, u128, u128, u16),
    net_dl_title: String,
    net_dl_key: (u64, u64),
    net_ul_title: String,
    net_ul_key: (u64, u64),

    // Modal system
    pub modal_manager: crate::ui::modal::ModalManager,

    // Connection state tracking
    pub connection_state: ConnectionState,
    last_connection_attempt: Instant,
    original_disconnect_time: Option<Instant>, // Track when we first disconnected
    connection_retry_count: u32,
    last_auto_retry: Option<Instant>, // Track last automatic retry
    replacement_connection: Option<socktop_connector::SocktopConnector>,
}

impl App {
    pub fn new() -> Self {
        Self {
            last_metrics: None,
            cpu_hist: VecDeque::with_capacity(600),
            cpu_hist_sum: 0,
            per_core_hist: PerCoreHistory::new(60),
            last_net_totals: None,
            last_net_sampled_at_ms: None,
            consecutive_request_timeouts: 0,
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
            selected_process_pid: None,
            selected_process_index: None,
            prev_selected_process_pid: None,
            process_search_active: false,
            process_search_query: String::new(),
            procs_filtered: Vec::new(),
            procs_filter_dirty: true,
            procs_row_cache: Vec::new(),
            procs_row_peak_cpu: 0.0,
            last_procs_poll: Instant::now()
                .checked_sub(Duration::from_secs(2))
                .unwrap_or_else(Instant::now), // trigger immediately on first loop
            last_disks_poll: Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now),
            procs_interval: Duration::from_secs(2),
            disks_interval: Duration::from_secs(5),
            metrics_interval: Duration::from_millis(500),
            process_details: None,
            journal_entries: None,
            process_cpu_history: VecDeque::with_capacity(600),
            process_cpu_history_sum: 0.0,
            process_mem_history: VecDeque::with_capacity(600),
            process_io_read_history: VecDeque::with_capacity(600),
            process_io_write_history: VecDeque::with_capacity(600),
            last_io_read_bytes: None,
            last_io_write_bytes: None,
            max_process_mem_bytes: 0,
            process_details_unsupported: false,
            last_process_details_poll: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            last_journal_poll: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            process_details_interval: Duration::from_millis(500),
            journal_interval: Duration::from_secs(5),
            ws_url: String::new(),
            tls_ca: None,
            verify_hostname: false,
            is_tls: false,
            has_token: false,
            force_compact: false,
            header_title: String::new(),
            header_intervals_text: String::new(),
            header_key: (String::new(), false, false, u128::MAX, u128::MAX, u16::MAX),
            net_dl_title: String::new(),
            net_dl_key: (u64::MAX, u64::MAX),
            net_ul_title: String::new(),
            net_ul_key: (u64::MAX, u64::MAX),
            modal_manager: ModalManager::new(),
            connection_state: ConnectionState::Disconnected,
            last_connection_attempt: Instant::now(),
            original_disconnect_time: None,
            connection_retry_count: 0,
            last_auto_retry: None,
            replacement_connection: None,
        }
    }

    /// Pins the compact layout at any window size (`--compact`).
    pub fn with_compact(mut self, force_compact: bool) -> Self {
        self.force_compact = force_compact;
        self
    }

    /// Pane rects for the current frame. The draw path and the mouse/key hit-testing
    /// paths all go through here so they cannot disagree about where a pane is.
    fn layout(&self, area: Rect) -> AppLayout {
        let has_gpu = self
            .last_metrics
            .as_ref()
            .and_then(|m| m.gpus.as_ref())
            .is_some_and(|g| !g.is_empty());
        compute_layout(area, self.force_compact, has_gpu)
    }

    pub fn with_intervals(mut self, metrics_ms: Option<u64>, procs_ms: Option<u64>) -> Self {
        metrics_ms.inspect(|&m| {
            self.metrics_interval = Duration::from_millis(m.max(MIN_METRICS_INTERVAL_MS));
        });
        procs_ms.inspect(|&p| {
            self.procs_interval = Duration::from_millis(p.max(MIN_PROCESSES_INTERVAL_MS));
        });
        self
    }

    pub fn with_status(mut self, is_tls: bool, has_token: bool) -> Self {
        self.is_tls = is_tls;
        self.has_token = has_token;
        self
    }

    /// Show a connection error modal
    pub fn show_connection_error(&mut self, message: String) {
        if !self.modal_manager.is_active() {
            self.connection_state = ConnectionState::Disconnected;
            // Set original disconnect time if this is the first disconnect
            if self.original_disconnect_time.is_none() {
                self.original_disconnect_time = Some(Instant::now());
            }
            self.modal_manager.push_modal(ModalType::ConnectionError {
                message,
                disconnected_at: self.original_disconnect_time.unwrap(),
                retry_count: self.connection_retry_count,
                auto_retry_countdown: self.seconds_until_next_auto_retry(),
            });
        }
    }

    /// Attempt to retry the connection
    pub async fn retry_connection(&mut self) {
        // This method is called from the normal event loop when connection is lost during operation
        self.connection_retry_count += 1;
        self.last_connection_attempt = Instant::now();
        self.connection_state = ConnectionState::Reconnecting;

        // Show retrying message
        if self.modal_manager.is_active() {
            self.modal_manager.pop_modal(); // Remove old modal
        }
        self.modal_manager.push_modal(ModalType::ConnectionError {
            message: "Retrying connection...".to_string(),
            disconnected_at: self
                .original_disconnect_time
                .unwrap_or(self.last_connection_attempt),
            retry_count: self.connection_retry_count,
            auto_retry_countdown: self.seconds_until_next_auto_retry(),
        });

        // Actually attempt to reconnect using stored parameters
        let tls_ca_ref = self.tls_ca.as_deref();
        match self
            .try_connect(&self.ws_url, tls_ca_ref, self.verify_hostname)
            .await
        {
            Ok(new_ws) => {
                // Connection successful! Store the new connection for the event loop to pick up
                self.replacement_connection = Some(new_ws);
                self.mark_connected();
                // The event loop will detect this and restart with the new connection
            }
            Err(e) => {
                // Connection failed, update modal with error
                self.modal_manager.pop_modal(); // Remove retrying modal
                self.modal_manager.push_modal(ModalType::ConnectionError {
                    message: format!("Retry failed: {e}"),
                    disconnected_at: self
                        .original_disconnect_time
                        .unwrap_or(self.last_connection_attempt),
                    retry_count: self.connection_retry_count,
                    auto_retry_countdown: self.seconds_until_next_auto_retry(),
                });
                self.connection_state = ConnectionState::Disconnected;
            }
        }
    }

    /// A request produced no reply in time. Any late reply would desync every
    /// subsequent request/response on this stream (replies are matched to
    /// requests purely by order), so treat the connection as poisoned and go
    /// through the standard reconnect flow — a fresh stream is realigned by
    /// construction.
    async fn poison_connection(&mut self, what: &str) {
        self.show_connection_error(format!("{what}; reconnecting…"));
        self.retry_connection().await;
    }

    /// Replace the connection WITHOUT any modal or state churn.
    ///
    /// For timeouts on the optional per-process endpoints: an old agent
    /// ignores those messages entirely (no late reply, so no desync), but a
    /// merely-slow agent would desync the stream — indistinguishable at
    /// timeout time, so we still swap to a fresh stream, silently. The
    /// ProcessDetails modal keeps showing its "Agent Update Required"
    /// message instead of being buried under a connection-error modal.
    /// Only a failed reconnect (connection genuinely dead) surfaces loudly.
    async fn quiet_reconnect(&mut self) {
        let tls_ca_ref = self.tls_ca.as_deref();
        match self
            .try_connect(&self.ws_url, tls_ca_ref, self.verify_hostname)
            .await
        {
            Ok(ws) => {
                self.replacement_connection = Some(ws);
            }
            Err(e) => {
                self.show_connection_error(format!("Reconnect failed: {e}"));
            }
        }
    }

    /// Mark connection as successful and dismiss any error modals
    pub fn mark_connected(&mut self) {
        if self.connection_state != ConnectionState::Connected {
            self.connection_state = ConnectionState::Connected;
            self.connection_retry_count = 0;
            self.original_disconnect_time = None; // Clear the original disconnect time
            self.last_auto_retry = None; // Clear auto retry timer
            // Remove connection error modal if it exists
            if self.modal_manager.is_active() {
                self.modal_manager.pop_modal();
            }
        }
    }

    /// Compute retry timing using pure policy function.
    fn current_retry_timing(&self) -> RetryTiming {
        compute_retry_timing(
            self.connection_state == ConnectionState::Disconnected,
            self.modal_manager.is_active(),
            self.original_disconnect_time,
            self.last_auto_retry,
            Instant::now(),
            Duration::from_secs(30),
        )
    }

    /// Check if we should perform an automatic retry (every 30 seconds)
    pub fn should_auto_retry(&self) -> bool {
        self.current_retry_timing().should_retry_now
    }

    /// Get seconds until next automatic retry (returns None if inactive)
    pub fn seconds_until_next_auto_retry(&self) -> Option<u64> {
        self.current_retry_timing().seconds_until_retry
    }

    /// Perform automatic retry
    pub async fn auto_retry_connection(&mut self) {
        self.last_auto_retry = Some(Instant::now());
        let tls_ca_ref = self.tls_ca.as_deref();

        // Increment retry count for auto retries too
        self.connection_retry_count += 1;

        // Show retrying modal
        self.modal_manager.pop_modal();
        self.modal_manager.push_modal(ModalType::ConnectionError {
            message: "Auto-retrying connection...".to_string(),
            disconnected_at: self.original_disconnect_time.unwrap_or(Instant::now()),
            retry_count: self.connection_retry_count,
            auto_retry_countdown: self.seconds_until_next_auto_retry(),
        });
        self.connection_state = ConnectionState::Reconnecting;

        // Attempt connection
        match self
            .try_connect(&self.ws_url, tls_ca_ref, self.verify_hostname)
            .await
        {
            Ok(new_ws) => {
                // Connection successful! Store the new connection for the event loop to pick up
                self.replacement_connection = Some(new_ws);
                self.mark_connected();
                // The event loop will detect this and restart with the new connection
            }
            Err(e) => {
                // Connection failed, update modal with error
                self.modal_manager.pop_modal(); // Remove retrying modal
                self.modal_manager.push_modal(ModalType::ConnectionError {
                    message: format!("Auto-retry failed: {e}"),
                    disconnected_at: self
                        .original_disconnect_time
                        .unwrap_or(self.last_connection_attempt),
                    retry_count: self.connection_retry_count,
                    auto_retry_countdown: self.seconds_until_next_auto_retry(),
                });
                self.connection_state = ConnectionState::Disconnected;
            }
        }
    }

    pub async fn run(
        &mut self,
        url: &str,
        tls_ca: Option<&str>,
        verify_hostname: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ws_url = url.to_string();
        self.tls_ca = tls_ca.map(|s| s.to_string());
        self.verify_hostname = verify_hostname;

        // Terminal setup first - so we can show connection error modals
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;

        // Try to connect to agent
        let ws = match self.try_connect(url, tls_ca, verify_hostname).await {
            Ok(connector) => connector,
            Err(e) => {
                // Show initial connection error and enter the error loop until user exits or we connect.
                self.show_connection_error(format!("Initial connection failed: {e}"));
                if let Err(err) = self
                    .run_with_connection_error(&mut terminal, url, tls_ca, verify_hostname)
                    .await
                {
                    // Terminal teardown then propagate error
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture
                    )?;
                    terminal.show_cursor()?;
                    return Err(err);
                }

                // If user chose to exit during error loop, mark quit and teardown.
                if self.should_quit || self.connection_state != ConnectionState::Connected {
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        DisableMouseCapture
                    )?;
                    terminal.show_cursor()?;
                    return Ok(());
                }

                // We should have a replacement connection after successful retry.
                match self.replacement_connection.take() {
                    Some(conn) => conn,
                    None => {
                        // Defensive: no connector despite Connected state; exit gracefully.
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;
                        return Ok(());
                    }
                }
            }
        };

        // Connection successful, mark as connected
        self.mark_connected();

        // Main loop
        let res = self.event_loop(&mut terminal, ws).await;

        // Teardown
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        res
    }

    /// Helper method to attempt connection
    async fn try_connect(
        &self,
        url: &str,
        tls_ca: Option<&str>,
        verify_hostname: bool,
    ) -> Result<SocktopConnector, Box<dyn std::error::Error>> {
        if let Some(ca_path) = tls_ca {
            Ok(connect_to_socktop_agent_with_tls(url, ca_path, verify_hostname).await?)
        } else {
            Ok(connect_to_socktop_agent(url).await?)
        }
    }

    /// Run the app with a connection error modal from the start
    async fn run_with_connection_error<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        _url: &str,
        _tls_ca: Option<&str>,
        _verify_hostname: bool,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        <B as ratatui::backend::Backend>::Error: 'static,
    {
        loop {
            // Handle input for modal
            while event::poll(Duration::from_millis(10))? {
                if let Event::Key(k) = event::read()? {
                    let action = self.modal_manager.handle_key(k.code);
                    match action {
                        ModalAction::ExitApp => {
                            return Ok(());
                        }
                        ModalAction::RetryConnection => {
                            // Show "Retrying..." message
                            self.modal_manager.pop_modal(); // Remove old modal
                            self.modal_manager.push_modal(ModalType::ConnectionError {
                                message: "Retrying connection...".to_string(),
                                disconnected_at: self
                                    .original_disconnect_time
                                    .unwrap_or(self.last_connection_attempt),
                                retry_count: self.connection_retry_count,
                                auto_retry_countdown: self.seconds_until_next_auto_retry(),
                            });

                            // Force a redraw to show the retrying message
                            terminal.draw(|f| self.draw(f))?;

                            // Update retry count
                            self.connection_retry_count += 1;
                            self.last_connection_attempt = Instant::now();

                            // Try to reconnect using stored parameters
                            let tls_ca_ref = self.tls_ca.as_deref();
                            match self
                                .try_connect(&self.ws_url, tls_ca_ref, self.verify_hostname)
                                .await
                            {
                                Ok(ws) => {
                                    // Connection successful!
                                    // Show success message briefly
                                    self.modal_manager.pop_modal(); // Remove retrying modal
                                    self.modal_manager.push_modal(ModalType::ConnectionError {
                                        message: "Connection restored! Starting...".to_string(),
                                        disconnected_at: self
                                            .original_disconnect_time
                                            .unwrap_or(self.last_connection_attempt),
                                        retry_count: self.connection_retry_count,
                                        auto_retry_countdown: self.seconds_until_next_auto_retry(),
                                    });
                                    terminal.draw(|f| self.draw(f))?;
                                    sleep(Duration::from_millis(500)).await; // Brief pause to show success

                                    // Explicitly clear all modals first
                                    while self.modal_manager.is_active() {
                                        self.modal_manager.pop_modal();
                                    }
                                    // Mark as connected (this also clears modals but let's be explicit)
                                    self.mark_connected();
                                    // Force a redraw to show the cleared state
                                    terminal.draw(|f| self.draw(f))?;
                                    // Start normal event loop
                                    return self.event_loop(terminal, ws).await;
                                }
                                Err(e) => {
                                    // Update modal with new error and retry count
                                    self.modal_manager.pop_modal(); // Remove retrying modal
                                    self.modal_manager.push_modal(ModalType::ConnectionError {
                                        message: format!("Retry failed: {e}"),
                                        disconnected_at: self
                                            .original_disconnect_time
                                            .unwrap_or(self.last_connection_attempt),
                                        retry_count: self.connection_retry_count,
                                        auto_retry_countdown: self.seconds_until_next_auto_retry(),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Check for automatic retry (every 30 seconds)
            if self.should_auto_retry() {
                self.auto_retry_connection().await;
                // If auto-retry succeeded, transition directly into the normal event loop
                if let Some(ws) = self.replacement_connection.take() {
                    // Ensure we are marked connected (auto_retry_connection already does this)
                    // Start the normal event loop using the newly established connection
                    return self.event_loop(terminal, ws).await;
                }
            }

            // Update countdown for connection error modal if active
            if self.modal_manager.is_active() {
                self.modal_manager
                    .update_connection_error_countdown(self.seconds_until_next_auto_retry());
            }

            // Draw the modal
            terminal.draw(|f| self.draw(f))?;
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn event_loop<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        mut ws: SocktopConnector,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        <B as ratatui::backend::Backend>::Error: 'static,
    {
        loop {
            // Main event loop
            let result = self.run_event_loop_iteration(terminal, &mut ws).await;

            // Check if we need to restart with a new connection
            if let Some(new_ws) = self.replacement_connection.take() {
                ws = new_ws;
                continue; // Restart the loop with new connection
            }

            // If we get here and there's no replacement, return the result
            return result;
        }
    }

    /// Drains and handles every queued terminal event (keys, mouse). Returns
    /// whether the caller must restart the event loop on a replacement
    /// connection. Extracted from the loop body so the tick wait can process
    /// input at ~30ms latency instead of letting it queue for a whole
    /// metrics interval.
    async fn drain_input<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<InputFlow, Box<dyn std::error::Error>>
    where
        <B as ratatui::backend::Backend>::Error: 'static,
    {
        // Drain everything already queued; the caller has verified (or will
        // verify via poll) that input is or may be pending.
        while event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(k) => {
                    // Handle modal input first - if a modal consumes the input, don't process normal keys
                    if self.modal_manager.is_active() {
                        let action = self.modal_manager.handle_key(k.code);
                        match action {
                            ModalAction::ExitApp => {
                                self.should_quit = true;
                                continue; // Skip normal key processing
                            }
                            ModalAction::RetryConnection => {
                                self.retry_connection().await;
                                // Check if retry succeeded and we have a replacement connection
                                if self.replacement_connection.is_some() {
                                    // Restart the outer loop on the new connection
                                    return Ok(InputFlow::RestartConnection);
                                }
                                continue; // Skip normal key processing
                            }
                            ModalAction::Cancel | ModalAction::Dismiss => {
                                // If ProcessDetails modal was dismissed, clear the data to save resources
                                if let Some(crate::ui::modal::ModalType::ProcessDetails {
                                    ..
                                }) = self.modal_manager.current_modal()
                                {
                                    self.clear_process_details();
                                }
                                // Modal was dismissed, skip normal key processing
                                continue;
                            }
                            ModalAction::Confirm => {
                                // Handle confirmation action here if needed in the future
                            }
                            ModalAction::SwitchToParentProcess(_current_pid) => {
                                // Get parent PID from current process details
                                if let Some(details) = &self.process_details
                                    && let Some(parent_pid) = details.process.parent_pid
                                {
                                    // Clear current process details
                                    self.clear_process_details();
                                    // Update selected process to parent
                                    self.selected_process_pid = Some(parent_pid);
                                    // Open modal for parent process
                                    self.modal_manager.push_modal(
                                        crate::ui::modal::ModalType::ProcessDetails {
                                            pid: parent_pid,
                                        },
                                    );
                                }
                                continue;
                            }
                            ModalAction::Handled => {
                                // Modal consumed the key, don't pass to main window
                                continue;
                            }
                            ModalAction::None => {
                                // Modal didn't handle the key, pass through to normal handling
                            }
                        }
                    }

                    // Handle search mode
                    if self.process_search_active {
                        match k.code {
                            KeyCode::Esc => {
                                // Exit search mode
                                self.process_search_active = false;
                                self.process_search_query.clear();
                                self.invalidate_procs_filter();
                                continue;
                            }
                            KeyCode::Enter => {
                                // Exit search mode, keep filter active, and auto-select first result
                                self.process_search_active = false;

                                // Auto-select first filtered result
                                let first = self.procs_filter().first().copied();
                                if let (Some(first_idx), Some(m)) =
                                    (first, self.last_metrics.as_ref())
                                {
                                    self.selected_process_index = Some(first_idx);
                                    self.selected_process_pid =
                                        Some(m.top_processes[first_idx].pid);
                                }
                                continue;
                            }
                            KeyCode::Backspace => {
                                self.process_search_query.pop();
                                self.invalidate_procs_filter();
                                continue;
                            }
                            KeyCode::Char(c) => {
                                self.process_search_query.push(c);
                                self.invalidate_procs_filter();
                                continue;
                            }
                            KeyCode::Up | KeyCode::Down => {
                                // Allow arrow keys to navigate even while in search mode
                                // Fall through to normal navigation handling
                            }
                            _ => {
                                continue; // Block other keys in search mode
                            }
                        }
                    }

                    // Normal key handling (only if no modal is active or modal didn't consume the key)
                    if matches!(
                        k.code,
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc
                    ) {
                        self.should_quit = true;
                    }

                    // Activate search mode on '/' (clears query if starting new search, or edits existing)
                    if matches!(k.code, KeyCode::Char('/')) {
                        self.process_search_active = true;
                        // Don't clear query - allow editing existing search
                        continue;
                    }

                    // Clear search filter on 'c' or 'C' (when not in search mode)
                    if matches!(k.code, KeyCode::Char('c') | KeyCode::Char('C'))
                        && !self.process_search_query.is_empty()
                        && !self.process_search_active
                    {
                        self.process_search_query.clear();
                        self.selected_process_pid = None;
                        self.selected_process_index = None;
                        self.invalidate_procs_filter();
                        continue;
                    }

                    // Show About modal on 'a' or 'A'
                    if matches!(k.code, KeyCode::Char('a') | KeyCode::Char('A')) {
                        self.modal_manager.push_modal(ModalType::About);
                    }

                    // Show Help modal on 'h' or 'H'
                    if matches!(k.code, KeyCode::Char('h') | KeyCode::Char('H')) {
                        self.modal_manager.push_modal(ModalType::Help);
                    }

                    // Per-core scroll via keys (Up/Down/PageUp/PageDown/Home/End)
                    let sz = terminal.size()?;
                    let area = Rect::new(0, 0, sz.width, sz.height);
                    let layout = self.layout(area);
                    let content = per_core_content_area(layout.per_core);

                    // Refresh the filtered+sorted index cache once before we
                    // borrow individual fields of `self`.
                    let _ = self.procs_filter();

                    // First try process selection (only handles arrows if a process is selected)
                    let process_handled = if self.last_procs_area.is_some() {
                        processes_handle_key_with_selection(ProcessKeyParams {
                            selected_process_pid: &mut self.selected_process_pid,
                            selected_process_index: &mut self.selected_process_index,
                            key: k,
                            metrics: self.last_metrics.as_ref(),
                            filtered_indices: &self.procs_filtered,
                        })
                    } else {
                        false
                    };

                    // If process selection didn't handle it, use CPU scrolling
                    if !process_handled {
                        per_core_handle_key(&mut self.per_core_scroll, k, content.height as usize);
                    }

                    // Auto-scroll to keep selected process visible
                    if let (Some(selected_idx), Some(p_area)) =
                        (self.selected_process_index, self.last_procs_area)
                        && self.last_metrics.is_some()
                    {
                        let idxs = &self.procs_filtered;

                        // Find the display position of the selected process in filtered list
                        if let Some(display_pos) = idxs.iter().position(|&idx| idx == selected_idx)
                        {
                            // Calculate viewport size
                            // Account for: borders (2) + header (1) + search box if active (3)
                            let extra_rows = if self.process_search_active
                                || !self.process_search_query.is_empty()
                            {
                                3 // search box with border
                            } else {
                                0
                            };
                            let viewport_rows =
                                p_area.height.saturating_sub(3 + extra_rows) as usize;

                            // Adjust scroll offset to keep selection visible
                            if display_pos < self.procs_scroll_offset {
                                // Selection is above viewport, scroll up
                                self.procs_scroll_offset = display_pos;
                            } else if display_pos >= self.procs_scroll_offset + viewport_rows {
                                // Selection is below viewport, scroll down
                                self.procs_scroll_offset =
                                    display_pos.saturating_sub(viewport_rows - 1);
                            }
                        }
                    }

                    // Check if process selection changed and clear details if so
                    if self.selected_process_pid != self.prev_selected_process_pid {
                        self.clear_process_details();
                        self.prev_selected_process_pid = self.selected_process_pid;
                    }

                    // Check if Enter was pressed with a process selected
                    if process_handled
                        && k.code == KeyCode::Enter
                        && let Some(selected_pid) = self.selected_process_pid
                    {
                        self.modal_manager
                            .push_modal(ModalType::ProcessDetails { pid: selected_pid });
                    }

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
                }
                Event::Mouse(m) => {
                    // If modal is active, don't handle mouse events on the main window
                    if self.modal_manager.is_active() {
                        continue;
                    }

                    // Layout to get areas
                    let sz = terminal.size()?;
                    let area = Rect::new(0, 0, sz.width, sz.height);
                    let layout = self.layout(area);

                    // Content wheel scrolling
                    let content = per_core_content_area(layout.per_core);
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
                        layout.per_core,
                        total_rows,
                    );

                    // Clamp to bounds
                    per_core_clamp(
                        &mut self.per_core_scroll,
                        total_rows,
                        content.height as usize,
                    );

                    // Refresh filter cache before partial borrows of self.
                    let _ = self.procs_filter();
                    let search_box_visible =
                        self.process_search_active || !self.process_search_query.is_empty();

                    // Processes table: sort by column on header click and handle row selection
                    if let (Some(_mm), Some(p_area)) =
                        (self.last_metrics.as_ref(), self.last_procs_area)
                    {
                        use crate::ui::processes::ProcessMouseParams;
                        let total_rows = self.procs_filtered.len();
                        if let Some(new_sort) =
                            processes_handle_mouse_with_selection(ProcessMouseParams {
                                scroll_offset: &mut self.procs_scroll_offset,
                                selected_process_pid: &mut self.selected_process_pid,
                                selected_process_index: &mut self.selected_process_index,
                                drag: &mut self.procs_drag,
                                mouse: m,
                                area: p_area,
                                total_rows,
                                metrics: self.last_metrics.as_ref(),
                                search_box_visible,
                                filtered_indices: &self.procs_filtered,
                            })
                        {
                            self.procs_sort_by = new_sort;
                            self.invalidate_procs_filter();
                        }
                    }

                    // Check if process selection changed via mouse and clear details if so
                    if self.selected_process_pid != self.prev_selected_process_pid {
                        self.clear_process_details();
                        self.prev_selected_process_pid = self.selected_process_pid;
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        Ok(InputFlow::Continue)
    }

    async fn run_event_loop_iteration<B: ratatui::backend::Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        ws: &mut SocktopConnector,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        <B as ratatui::backend::Backend>::Error: 'static,
    {
        loop {
            // Input: drain anything already queued
            if matches!(
                self.drain_input(terminal).await?,
                InputFlow::RestartConnection
            ) {
                return Ok(());
            }

            // Check for automatic retry (every 30 seconds)
            if self.should_auto_retry() {
                self.auto_retry_connection().await;
                // Check if retry succeeded and we have a replacement connection
                if self.replacement_connection.is_some() {
                    // Signal that we want to restart with new connection
                    return Ok(());
                }
            }

            if self.should_quit {
                break;
            }

            // Paint the current state BEFORE fetching: a request can stall for
            // the full 5s timeout, and an iteration that ends in a poisoned-
            // stream restart never reaches the draw at the bottom — without
            // this, an agent that never answers left the screen permanently
            // blank. (ratatui diffs make an unchanged repaint nearly free.)
            terminal.draw(|f| self.draw(f))?;

            // Fetch and update. Skipped while disconnected — the retry paths
            // (manual 'r' or the 30s auto-retry) own recovery, and hammering a
            // dead socket with 5s-timeout requests would stall the loop. The
            // shared draw + responsive wait below still run, so the error
            // modal stays live and input stays snappy.
            if self.connection_state == ConnectionState::Connected {
                match timeout(REQUEST_TIMEOUT, ws.request(AgentRequest::Metrics)).await {
                    Err(_) => {
                        self.consecutive_request_timeouts += 1;
                        if self.consecutive_request_timeouts >= 2 {
                            // The agent accepts connections but never answers
                            // (wrong protocol era, or wedged): reconnecting
                            // can't help, so surface a persistent error and
                            // leave recovery to the manual/auto retry paths.
                            self.show_connection_error(
                                "Agent is not responding to requests".to_string(),
                            );
                        } else {
                            self.poison_connection("Metrics request timed out").await;
                        }
                    }
                    Ok(Ok(AgentResponse::Metrics(m))) => {
                        self.mark_connected(); // Mark as connected on successful request
                        self.consecutive_request_timeouts = 0;
                        self.update_with_metrics(m);

                        // Only poll processes every 2s
                        if self.last_procs_poll.elapsed() >= self.procs_interval {
                            let mut updated = false;
                            match timeout(REQUEST_TIMEOUT, ws.request(AgentRequest::Processes))
                                .await
                            {
                                Err(_) => {
                                    self.poison_connection("Processes request timed out").await;
                                }
                                Ok(Ok(AgentResponse::Processes(procs))) => {
                                    if let Some(mm) = self.last_metrics.as_mut() {
                                        mm.top_processes = procs.top_processes;
                                        mm.process_count = Some(procs.process_count);
                                        updated = true;
                                    }
                                }
                                // Request error or wrong type: keep stale rows; a
                                // broken socket surfaces on the next metrics tick.
                                Ok(_) => {}
                            }
                            if updated {
                                self.invalidate_procs_filter();
                                // Rebuild the pre-formatted row cache for the next
                                // ~N frames. Done once per poll, not per frame.
                                if let Some(mm) = self.last_metrics.as_ref() {
                                    self.procs_row_peak_cpu =
                                        crate::ui::processes::rebuild_row_cache(
                                            mm,
                                            &mut self.procs_row_cache,
                                        );
                                }
                            }
                            self.last_procs_poll = Instant::now();
                        }

                        // Only poll disks every 5s
                        if self.connection_state == ConnectionState::Connected
                            && self.last_disks_poll.elapsed() >= self.disks_interval
                        {
                            match timeout(REQUEST_TIMEOUT, ws.request(AgentRequest::Disks)).await {
                                Err(_) => {
                                    self.poison_connection("Disks request timed out").await;
                                }
                                Ok(Ok(AgentResponse::Disks(mut disks))) => {
                                    if let Some(mm) = self.last_metrics.as_mut() {
                                        dedup_disks(&mut disks);
                                        mm.disks = disks;
                                    }
                                }
                                Ok(_) => {}
                            }
                            self.last_disks_poll = Instant::now();
                        }

                        // Poll process details when modal is active and process is selected
                        if let Some(pid) = self.selected_process_pid
                            && self.connection_state == ConnectionState::Connected
                        {
                            // Check if ProcessDetails modal is currently active
                            if let Some(crate::ui::modal::ModalType::ProcessDetails { .. }) =
                                self.modal_manager.current_modal()
                            {
                                // Poll process details every 500ms when modal is
                                // active. Skipped once the agent is known not to
                                // support the endpoint (flag resets when the modal
                                // closes or the selection changes, so a one-off
                                // timeout doesn't disable details for the session).
                                if self.connection_state == ConnectionState::Connected
                                    && !self.process_details_unsupported
                                    && self.last_process_details_poll.elapsed()
                                        >= self.process_details_interval
                                {
                                    // Use timeout to prevent blocking the event loop
                                    match timeout(
                                        Duration::from_millis(2000),
                                        ws.request(AgentRequest::ProcessMetrics { pid }),
                                    )
                                    .await
                                    {
                                        Ok(Ok(AgentResponse::ProcessMetrics(details))) => {
                                            // Update history for sparklines
                                            let cpu_usage = details.process.cpu_usage;
                                            let evicted_cpu = push_capped(
                                                &mut self.process_cpu_history,
                                                cpu_usage,
                                                600,
                                            );
                                            self.process_cpu_history_sum =
                                                self.process_cpu_history_sum + cpu_usage
                                                    - evicted_cpu.unwrap_or(0.0);

                                            let mem_bytes = details.process.mem_bytes;
                                            push_capped(
                                                &mut self.process_mem_history,
                                                mem_bytes,
                                                600,
                                            );

                                            // Track maximum memory usage
                                            if mem_bytes > self.max_process_mem_bytes {
                                                self.max_process_mem_bytes = mem_bytes;
                                            }

                                            // I/O bytes from agent are cumulative, calculate deltas
                                            if let Some(read) = details.process.read_bytes {
                                                let delta =
                                                    if let Some(last) = self.last_io_read_bytes {
                                                        read.saturating_sub(last)
                                                    } else {
                                                        0 // First sample, no delta available
                                                    };
                                                push_capped(
                                                    &mut self.process_io_read_history,
                                                    delta,
                                                    600,
                                                );
                                                self.last_io_read_bytes = Some(read);
                                            }
                                            if let Some(write) = details.process.write_bytes {
                                                let delta =
                                                    if let Some(last) = self.last_io_write_bytes {
                                                        write.saturating_sub(last)
                                                    } else {
                                                        0 // First sample, no delta available
                                                    };
                                                push_capped(
                                                    &mut self.process_io_write_history,
                                                    delta,
                                                    600,
                                                );
                                                self.last_io_write_bytes = Some(write);
                                            }

                                            self.process_details = Some(details);
                                            self.process_details_unsupported = false;
                                        }
                                        Ok(Err(_)) => {
                                            // Agent responded with an error: endpoint
                                            // not supported.
                                            self.process_details_unsupported = true;
                                        }
                                        Err(_) => {
                                            // No reply at all: old agents IGNORE
                                            // this message, so show the "Agent
                                            // Update Required" state and refresh
                                            // the stream quietly (a merely-slow
                                            // agent's late reply would otherwise
                                            // desync it).
                                            self.process_details_unsupported = true;
                                            self.quiet_reconnect().await;
                                        }
                                        Ok(Ok(_)) => {
                                            // Wrong response type
                                            self.process_details_unsupported = true;
                                        }
                                    }
                                    self.last_process_details_poll = Instant::now();
                                }

                                // Poll journal entries every 5s when modal is active.
                                // Gated on the same unsupported flag: agents that lack
                                // process details lack the journal endpoint too.
                                if self.connection_state == ConnectionState::Connected
                                    && !self.process_details_unsupported
                                    && self.last_journal_poll.elapsed() >= self.journal_interval
                                {
                                    // Use timeout to prevent blocking the event loop
                                    match timeout(
                                        Duration::from_millis(2000),
                                        ws.request(AgentRequest::JournalEntries { pid }),
                                    )
                                    .await
                                    {
                                        Ok(Ok(AgentResponse::JournalEntries(journal))) => {
                                            self.journal_entries = Some(journal);
                                        }
                                        Err(_) => {
                                            // No reply: same quiet stream refresh
                                            // as the details endpoint above.
                                            self.quiet_reconnect().await;
                                        }
                                        Ok(Err(_)) | Ok(Ok(_)) => {
                                            // Endpoint unsupported or wrong type;
                                            // keep journal_entries as None
                                        }
                                    }
                                    self.last_journal_poll = Instant::now();
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        // Connection error - show modal if not already shown
                        let error_message = format!("Failed to fetch metrics: {e}");
                        self.show_connection_error(error_message);
                    }
                    Ok(_) => {
                        // Unexpected response type
                        self.show_connection_error("Unexpected response from agent".to_string());
                    }
                }
            }

            // A poisoned connection may have been replaced mid-iteration:
            // restart on the fresh stream before issuing any more requests.
            if self.replacement_connection.is_some() {
                return Ok(());
            }

            // Update countdown for connection error modal if active
            if self.modal_manager.is_active() {
                self.modal_manager
                    .update_connection_error_countdown(self.seconds_until_next_auto_retry());
            }

            // Draw
            terminal.draw(|f| self.draw(f))?;

            // Tick wait, kept responsive: instead of sleeping the whole
            // metrics interval (which queued keys/wheel events for up to
            // 500ms and applied them in bursts), wait in ≤33ms slices and
            // handle + repaint input the moment it arrives.
            let deadline = Instant::now() + self.metrics_interval;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() || self.should_quit {
                    break;
                }
                if !event::poll(remaining.min(Duration::from_millis(33)))? {
                    continue;
                }
                if matches!(
                    self.drain_input(terminal).await?,
                    InputFlow::RestartConnection
                ) {
                    return Ok(());
                }
                terminal.draw(|f| self.draw(f))?;
            }
        }

        Ok(())
    }

    /// Mark the filtered-process cache stale. Call this whenever
    /// `procs_sort_by`, `process_search_query`, or the top_processes content
    /// changes — the cache is rebuilt lazily on the next read.
    pub fn invalidate_procs_filter(&mut self) {
        self.procs_filter_dirty = true;
    }

    /// Lazily refresh and return the cached filtered+sorted process indices.
    /// Empty slice when there are no metrics yet.
    pub fn procs_filter(&mut self) -> &[usize] {
        if self.procs_filter_dirty {
            self.procs_filtered.clear();
            if let Some(m) = self.last_metrics.as_ref() {
                crate::ui::processes::fill_filtered_sorted_indices(
                    m,
                    &self.process_search_query,
                    self.procs_sort_by,
                    &mut self.procs_filtered,
                );
            }
            self.procs_filter_dirty = false;
        }
        &self.procs_filtered
    }

    /// Clear process details when modal is closed or selection changes
    pub fn clear_process_details(&mut self) {
        self.process_details = None;
        self.journal_entries = None;
        self.process_cpu_history.clear();
        self.process_cpu_history_sum = 0.0;
        self.process_mem_history.clear();
        self.process_io_read_history.clear();
        self.process_io_write_history.clear();
        self.last_io_read_bytes = None;
        self.last_io_write_bytes = None;
        self.max_process_mem_bytes = 0;
        self.process_details_unsupported = false;
    }

    fn update_with_metrics(&mut self, mut m: Metrics) {
        if let Some(prev) = self.last_metrics.as_mut() {
            // Preserve slower fields when the fast payload omits them.
            // prev is about to be dropped so we can move its Vecs instead of cloning.
            if m.disks.is_empty() {
                m.disks = std::mem::take(&mut prev.disks);
            }
            if m.top_processes.is_empty() {
                m.top_processes = std::mem::take(&mut prev.top_processes);
            }
            if m.process_count.is_none() {
                m.process_count = prev.process_count;
            }
        }

        // CPU avg history with running sum
        let v = m.cpu_total.clamp(0.0, 100.0).round() as u64;
        let evicted = push_capped(&mut self.cpu_hist, v, 600);
        self.cpu_hist_sum = self.cpu_hist_sum + v - evicted.unwrap_or(0);

        // Per-core history (push current samples)
        self.per_core_hist.ensure_cores(m.cpu_per_core.len());
        self.per_core_hist.push_samples(&m.cpu_per_core);

        // NET: sum across all ifaces, compute KB/s. Prefer the agent's sample
        // timestamps (the agent serves TTL-cached snapshots, so client receive
        // time overstates dt on a cache hit and produces a 0-then-2x sawtooth);
        // fall back to the client clock against pre-1.60 agents.
        let now = Instant::now();
        let rx_total = m.networks.iter().map(|n| n.received).sum::<u64>();
        let tx_total = m.networks.iter().map(|n| n.transmitted).sum::<u64>();
        let (rx_kb, tx_kb) = if let Some((prx, ptx, pts)) = self.last_net_totals {
            // None = identical agent snapshot (cache hit): repeat the previous
            // rates so the timeline advances without a fake dip to zero.
            let dt = match (m.sampled_at_ms, self.last_net_sampled_at_ms) {
                (Some(a), Some(b)) if a == b => None,
                (Some(a), Some(b)) if a > b => Some((a - b) as f64 / 1000.0),
                // Agent restarted or clock stepped backwards: client clock.
                _ => Some(now.duration_since(pts).as_secs_f64().max(1e-6)),
            };
            match dt {
                None => (
                    self.rx_hist.back().copied().unwrap_or(0),
                    self.tx_hist.back().copied().unwrap_or(0),
                ),
                Some(dt) => {
                    let dt = dt.max(1e-6);
                    let rx = ((rx_total.saturating_sub(prx)) as f64 / dt / 1024.0).round() as u64;
                    let tx = ((tx_total.saturating_sub(ptx)) as f64 / dt / 1024.0).round() as u64;
                    (rx, tx)
                }
            }
        } else {
            (0, 0)
        };
        self.last_net_totals = Some((rx_total, tx_total, now));
        self.last_net_sampled_at_ms = m.sampled_at_ms;
        push_capped(&mut self.rx_hist, rx_kb, 600);
        push_capped(&mut self.tx_hist, tx_kb, 600);
        self.rx_peak = self.rx_peak.max(rx_kb);
        self.tx_peak = self.tx_peak.max(tx_kb);

        // Store merged snapshot
        self.last_metrics = Some(m);
    }

    pub fn draw(&mut self, f: &mut ratatui::Frame<'_>) {
        let area = f.area();
        let l = self.layout(area);

        // Header — refresh cached strings only when their inputs change so the
        // ratatui diff renderer can suppress repaints on idle frames. The wording now
        // depends on the row width too, so that is part of the key.
        {
            let hostname = self.last_metrics.as_ref().map(|mm| mm.hostname.as_str());
            let state = HeaderState {
                hostname,
                is_tls: self.is_tls,
                has_token: self.has_token,
                metrics_ms: self.metrics_interval.as_millis(),
                procs_ms: self.procs_interval.as_millis(),
            };
            let key = (
                hostname.unwrap_or("").to_string(),
                self.is_tls,
                self.has_token,
                state.metrics_ms,
                state.procs_ms,
                l.header.width,
            );
            if self.header_key != key {
                let (title, intervals) = build_header(state, l.header.width);
                self.header_title = title;
                self.header_intervals_text = intervals;
                self.header_key = key;
            }
        }
        draw_header(f, l.header, &self.header_title, &self.header_intervals_text);

        draw_cpu_avg_graph(
            f,
            l.cpu,
            &mut self.cpu_hist,
            self.cpu_hist_sum,
            self.last_metrics.as_ref(),
        );
        draw_per_core_bars(
            f,
            l.per_core,
            self.last_metrics.as_ref(),
            &mut self.per_core_hist,
            self.per_core_scroll,
        );

        // Memory + Swap: stacked vertically in the normal layout, side by side in the
        // row Disks vacates in compact mode.
        draw_mem(f, l.mem, self.last_metrics.as_ref());
        draw_swap(f, l.swap, self.last_metrics.as_ref());

        // GPU: a panel beside Memory/Swap normally, a single full-width line in compact
        // mode, and absent entirely when the host reports no GPU while compact.
        if let Some(gpu_area) = l.gpu {
            if l.mode.is_compact() {
                draw_gpu_compact(f, gpu_area, self.last_metrics.as_ref());
            } else {
                draw_gpu(f, gpu_area, self.last_metrics.as_ref());
            }
        }

        if let Some(disks_area) = l.disks {
            draw_disks(f, disks_area, self.last_metrics.as_ref());
        }

        // Net titles only change when the throughput or peak changes.
        let rx_now = self.rx_hist.back().copied().unwrap_or(0);
        let rx_key = (rx_now, self.rx_peak);
        if self.net_dl_key != rx_key {
            self.net_dl_title = format!("Download (KB/s) — now: {rx_now} | peak: {}", self.rx_peak);
            self.net_dl_key = rx_key;
        }
        draw_net_spark(
            f,
            l.download,
            &self.net_dl_title,
            &mut self.rx_hist,
            ratatui::style::Color::Green,
        );

        let tx_now = self.tx_hist.back().copied().unwrap_or(0);
        let tx_key = (tx_now, self.tx_peak);
        if self.net_ul_key != tx_key {
            self.net_ul_title = format!("Upload (KB/s) — now: {tx_now} | peak: {}", self.tx_peak);
            self.net_ul_key = tx_key;
        }
        draw_net_spark(
            f,
            l.upload,
            &self.net_ul_title,
            &mut self.tx_hist,
            ratatui::style::Color::Blue,
        );

        // Right bottom: Top Processes fills the column
        let procs_area = l.procs;
        // Cache for input handlers
        self.last_procs_area = Some(procs_area);
        // Refresh the filter cache before partial borrows of self.
        let _ = self.procs_filter();
        crate::ui::processes::draw_top_processes(
            f,
            procs_area,
            crate::ui::processes::ProcessDisplayParams {
                metrics: self.last_metrics.as_ref(),
                scroll_offset: self.procs_scroll_offset,
                sort_by: self.procs_sort_by,
                selected_process_pid: self.selected_process_pid,
                selected_process_index: self.selected_process_index,
                search_query: &self.process_search_query,
                search_active: self.process_search_active,
                filtered_indices: &self.procs_filtered,
                cached_rows: &self.procs_row_cache,
                peak_cpu: self.procs_row_peak_cpu,
            },
        );

        // Render modals on top of everything else
        if self.modal_manager.is_active() {
            use crate::ui::modal::{ProcessHistoryData, ProcessModalData};
            self.modal_manager.render(
                f,
                ProcessModalData {
                    details: self.process_details.as_ref(),
                    journal: self.journal_entries.as_ref(),
                    history: ProcessHistoryData {
                        cpu: &self.process_cpu_history,
                        cpu_sum: self.process_cpu_history_sum,
                        mem: &self.process_mem_history,
                        io_read: &self.process_io_read_history,
                        io_write: &self.process_io_write_history,
                    },
                    max_mem_bytes: self.max_process_mem_bytes,
                    unsupported: self.process_details_unsupported,
                },
            );
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
