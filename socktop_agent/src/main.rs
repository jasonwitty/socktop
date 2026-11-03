//! socktop agent entrypoint: sets up sysinfo handles and serves a WebSocket endpoint at /ws.

mod gpu;
mod metrics;
mod proto;
// sampler module removed (metrics now purely request-driven)
mod state;
mod types;
mod ws;

use axum::{Router, http::StatusCode, routing::get};
use std::net::SocketAddr;
use std::str::FromStr;

mod tls;

use state::AppState;

fn arg_flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}
fn arg_value(name: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == name {
            return it.next();
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    // Name the crypto provider before any TLS operation. `rustls` is compiled
    // with exactly one provider (ring, from the workspace manifest), so this is
    // belt-and-braces: it says out loud which one, and it stops compiling if the
    // workspace ever switches.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok(); // Ignore error if already installed

    #[cfg(feature = "logging")]
    tracing_subscriber::fmt::init();

    // Runtime shape: one worker thread, no pool.
    //
    // The agent is I/O-bound (WebSocket, /proc reads, sysinfo) and every request
    // path funnels through locks that a poll already holds, so a second worker
    // thread cannot overlap useful work — it only adds a hand-off between threads
    // and costs a stack. Measured on a Pi 5: the two worker threads were the
    // process's largest single chunk of private memory, 1.6 MB of the 3.9 MB
    // resident at startup (8 MB of virtual each; the kernel reuses their idle
    // stacks, so resident cost shows up mainly as RSS, not VmPeak).
    //
    // The blocking pool stays, and the two slow collectors still move to it: the
    // journalctl subprocess (12 ms wall / 10 ms child CPU for 100 entries on a
    // Pi) and NVML. `spawn_blocking` works on this runtime.
    //
    // Override: SOCKTOP_WORKER_THREADS=N switches back to a multi-thread runtime
    // with N workers, for a deployment with many clients on a many-core box.
    let worker_threads = std::env::var("SOCKTOP_WORKER_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .map(|n| n.clamp(1, 16));

    let runtime = match worker_threads {
        Some(n) => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(n)
            .thread_name("socktop-agent")
            .enable_all()
            .build()?,
        None => tokio::runtime::Builder::new_current_thread()
            .thread_name("socktop-agent")
            .enable_all()
            .build()?,
    };

    runtime.block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    // Version flag (print and exit). Keep before heavy initialization.
    if arg_flag("--version") || arg_flag("-V") {
        println!("socktop_agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let state = AppState::new();

    // No background samplers: metrics collected on-demand per websocket request.

    // Web app: route /ws to the websocket handler
    async fn healthz() -> StatusCode {
        println!("/healthz request");
        StatusCode::OK
    }
    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/healthz", get(healthz))
        .with_state(state.clone());

    let enable_ssl =
        arg_flag("--enableSSL") || std::env::var("SOCKTOP_ENABLE_SSL").ok().as_deref() == Some("1");
    if enable_ssl {
        // Port can be overridden by --port or SOCKTOP_PORT; default to 8443 when SSL
        let port = arg_value("--port")
            .or_else(|| arg_value("-p"))
            .or_else(|| std::env::var("SOCKTOP_PORT").ok())
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(8443);

        let (cert_path, key_path) = tls::ensure_self_signed_cert()?;
        let cfg = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_path, key_path).await?;

        let addr = SocketAddr::from_str(&format!("0.0.0.0:{port}"))?;
        println!("socktop_agent: TLS enabled. Listening on wss://{addr}/ws");
        axum_server::bind_rustls(addr, cfg)
            .serve(app.into_make_service())
            .await?;
        return Ok(());
    }

    // Non-TLS HTTP/WS path
    let port = arg_value("--port")
        .or_else(|| arg_value("-p"))
        .or_else(|| std::env::var("SOCKTOP_PORT").ok())
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("socktop_agent: Listening on ws://{addr}/ws");
    axum_server::bind(addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

// Unit tests for CLI parsing moved to `tests/port_parse.rs`.
