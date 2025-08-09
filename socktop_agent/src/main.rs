//! socktop agent entrypoint: sets up sysinfo handles, launches a sampler,
//! and serves a WebSocket endpoint at /ws.

mod metrics;
mod sampler;
mod state;
mod types;
mod ws;

use axum::{routing::get, Router};
use std::{
    collections::HashMap, net::SocketAddr, sync::atomic::AtomicUsize, sync::Arc, time::Duration,
};
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind,
    RefreshKind, System,
};
use tokio::sync::{Mutex, Notify, RwLock};
use tracing_subscriber::EnvFilter;

use sampler::spawn_sampler;
use state::{AppState, SharedTotals};
use ws::ws_handler;

#[tokio::main]
async fn main() {
    // Init logging; configure with RUST_LOG (e.g., RUST_LOG=info).
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    // sysinfo build specifics (scopes what refresh_all() will touch internally)
    let refresh_kind = RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::everything())
        .with_memory(MemoryRefreshKind::everything())
        .with_processes(ProcessRefreshKind::everything());

    // Initialize sysinfo handles once and keep them alive
    let mut sys = System::new_with_specifics(refresh_kind);
    sys.refresh_all();

    let mut nets = Networks::new();
    nets.refresh(true);

    let mut components = Components::new();
    components.refresh(true);

    let mut disks = Disks::new();
    disks.refresh(true);

    // Shared state across requests
    let state = AppState {
        sys: Arc::new(Mutex::new(sys)),
        nets: Arc::new(Mutex::new(nets)),
        net_totals: Arc::new(Mutex::new(HashMap::<String, (u64, u64)>::new())) as SharedTotals,
        components: Arc::new(Mutex::new(components)),
        disks: Arc::new(Mutex::new(disks)),
        last_json: Arc::new(RwLock::new(String::new())),
        // new: adaptive sampling controls
        client_count: Arc::new(AtomicUsize::new(0)),
        wake_sampler: Arc::new(Notify::new()),
        auth_token: std::env::var("SOCKTOP_TOKEN")
            .ok()
            .filter(|s| !s.is_empty()),
    };

    // Start background sampler (adjust cadence as needed)
    let _sampler = spawn_sampler(state.clone(), Duration::from_millis(500));

    // Web app
    let port = resolve_port();
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    //output to console
    println!("Remote agent running at http://{}", addr);
    println!("WebSocket endpoint: ws://{}/ws", addr);

    //trace logging
    tracing::info!("Remote agent running at http://{} (ws at /ws)", addr);
    tracing::info!("WebSocket endpoint: ws://{}/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// Resolve the listening port from CLI args/env with a 3000 default.
// Supports: --port <PORT>, -p <PORT>, a bare numeric positional arg, or SOCKTOP_PORT.
fn resolve_port() -> u16 {
    const DEFAULT: u16 = 3000;

    // Env takes precedence over positional, but is overridden by explicit flags if present.
    if let Ok(s) = std::env::var("SOCKTOP_PORT") {
        if let Ok(p) = s.parse::<u16>() {
            if p != 0 {
                return p;
            }
        }
        eprintln!(
            "Warning: invalid SOCKTOP_PORT='{}'; using default {}",
            s, DEFAULT
        );
    }

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                if let Some(v) = args.next() {
                    match v.parse::<u16>() {
                        Ok(p) if p != 0 => return p,
                        _ => {
                            eprintln!("Invalid port '{}'; using default {}", v, DEFAULT);
                            return DEFAULT;
                        }
                    }
                } else {
                    eprintln!("Missing value for {} ; using default {}", arg, DEFAULT);
                    return DEFAULT;
                }
            }
            "--help" | "-h" => {
                println!("Usage: socktop_agent [--port <PORT>] [PORT]\n       SOCKTOP_PORT=<PORT> socktop_agent");
                std::process::exit(0);
            }
            s => {
                if let Ok(p) = s.parse::<u16>() {
                    if p != 0 {
                        return p;
                    }
                }
            }
        }
    }

    DEFAULT
}
