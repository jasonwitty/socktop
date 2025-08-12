//! socktop agent entrypoint: sets up sysinfo handles, launches a sampler,
//! and serves a WebSocket endpoint at /ws.

mod gpu;
mod metrics;
mod sampler;
mod state;
mod types;
mod ws;

use axum::{routing::get, Router};
use std::net::SocketAddr;

use crate::sampler::{spawn_disks_sampler, spawn_process_sampler, spawn_sampler};
use state::AppState;
use ws::ws_handler;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = AppState::new();

    // Start background sampler (adjust cadence as needed)
    // 500ms fast metrics
    let _h_fast = spawn_sampler(state.clone(), std::time::Duration::from_millis(500));
    // 2s processes (top 50)
    let _h_procs = spawn_process_sampler(state.clone(), std::time::Duration::from_secs(2), 50);
    // 5s disks
    let _h_disks = spawn_disks_sampler(state.clone(), std::time::Duration::from_secs(5));

    // Web app
    let port = resolve_port();
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    //output to console
    println!("Remote agent running at http://{addr}");
    println!("WebSocket endpoint: ws://{addr}/ws");

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
        eprintln!("Warning: invalid SOCKTOP_PORT='{s}'; using default {DEFAULT}");
    }

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                if let Some(v) = args.next() {
                    match v.parse::<u16>() {
                        Ok(p) if p != 0 => return p,
                        _ => {
                            eprintln!("Invalid port '{v}'; using default {DEFAULT}");
                            return DEFAULT;
                        }
                    }
                } else {
                    eprintln!("Missing value for {arg} ; using default {DEFAULT}");
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
