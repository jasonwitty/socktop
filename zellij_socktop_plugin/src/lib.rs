//! Socktop Zellij plugin.
//!
//! Note on the SDK API: `register_plugin!` (zellij-tile >= 0.40) already emits a
//! `thread_local! { static STATE: RefCell<State> }` for us, and calls the trait
//! methods with `&mut self` bound to that same state. So the plugin must NOT
//! declare its own `static mut STATE`; it just uses the fields of `self`.

use std::collections::BTreeMap;

use socktop_connector::{AgentRequest, AgentResponse, ConnectorConfig, SocktopConnector};
use zellij_tile::prelude::*;

#[derive(Default)]
struct State {
    connector: Option<SocktopConnector>,
    metrics_data: Option<String>,
    connection_status: String,
    error_message: Option<String>,
    update_counter: u32,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        // Get server URL from plugin config or use default
        let server_url = configuration
            .get("server_url")
            .cloned()
            .unwrap_or_else(|| "ws://localhost:3000/ws".to_string());

        // Initialize connector configuration
        let config = ConnectorConfig::new(&server_url);
        self.connector = Some(SocktopConnector::new(config));
        self.connection_status = "Connecting...".to_string();

        // Set up periodic updates
        set_timeout(1.0); // Update every second

        // Start initial connection
        self.connect_to_socktop();

        request_permission(&[PermissionType::ReadApplicationState]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Timer(_) => {
                self.update_counter += 1;

                // Request metrics every update cycle
                self.fetch_metrics();

                // Set next timer
                set_timeout(2.0); // Update every 2 seconds
                true
            }
            Event::Key(key) => match key.bare_key {
                // Reconnect on 'r' key press (ignore modified chords).
                BareKey::Char('r') | BareKey::Char('R') if key.key_modifiers.is_empty() => {
                    self.connect_to_socktop();
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, _cols: usize) {
        let mut output = Vec::new();

        // Header
        output.push("╭─ Socktop Metrics Plugin ─╮".to_string());
        output.push(format!(
            "│ Status: {:<18} │",
            if self.connection_status.is_empty() {
                "-"
            } else {
                &self.connection_status
            }
        ));
        output.push("├──────────────────────────╯".to_string());

        // Metrics display
        if let Some(metrics) = self.metrics_data.as_ref() {
            output.push("│ System Metrics:".to_string());
            output.push(format!("│   {metrics}"));
        } else if let Some(error) = self.error_message.as_ref() {
            output.push("│ Error:".to_string());
            output.push(format!("│   {error}"));
        } else {
            output.push("│ Waiting for data...".to_string());
        }

        // Footer
        output.push("│".to_string());
        output.push(format!(
            "│ Updates: {} │ Press 'r' to reconnect",
            self.update_counter
        ));
        output.push("╰──────────────────────────╯".to_string());

        // Print lines within terminal bounds
        for (i, line) in output.iter().enumerate() {
            if i < rows {
                println!("{line}");
            }
        }
    }
}

impl State {
    fn connect_to_socktop(&mut self) {
        if self.connector.is_some() {
            self.connection_status = "Connecting...".to_string();
            self.error_message = None;
        } else {
            self.connection_status = "Disconnected".to_string();
            self.error_message = Some("No connector available".to_string());
        }
    }

    fn fetch_metrics(&mut self) {
        if self.connector.is_none() {
            self.error_message = Some("No connector available".to_string());
            self.connection_status = "Disconnected".to_string();
            return;
        }

        match self.try_get_metrics() {
            Ok(metrics_text) => {
                self.metrics_data = Some(metrics_text);
                self.connection_status = "Active".to_string();
                self.error_message = None;
            }
            Err(error) => {
                self.error_message = Some(error);
                self.connection_status = "Error".to_string();
            }
        }
    }

    /// Blocking fetch: Zellij gives plugins no async runtime, so the connector
    /// future is driven by a local executor. Because the plugin is the only
    /// thing running on the wasm thread this is acceptable for a single
    /// request/response round trip, but it does block rendering for its
    /// duration.
    fn try_get_metrics(&mut self) -> Result<String, String> {
        let connector = self
            .connector
            .as_mut()
            .ok_or_else(|| "No connector available".to_string())?;

        if let Err(e) = futures::executor::block_on(connector.connect()) {
            return Err(format!("Connection failed: {e}"));
        }

        match futures::executor::block_on(connector.request(AgentRequest::Metrics)) {
            Ok(AgentResponse::Metrics(metrics)) => Ok(format!(
                "CPU: {:.1}% | Mem: {:.1}% | Host: {} | Procs: {}",
                metrics.cpu_total,
                (metrics.mem_used as f64 / metrics.mem_total as f64) * 100.0,
                metrics.hostname,
                metrics.top_processes.len()
            )),
            Ok(_) => Err("Unexpected response type".to_string()),
            Err(e) => Err(format!("Request failed: {e}")),
        }
    }
}
