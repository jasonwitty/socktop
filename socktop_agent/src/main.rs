//! socktop agent entrypoint: sets up sysinfo handles, launches a sampler,
//! and serves a WebSocket endpoint at /ws.

mod gpu;
mod metrics;
mod proto;
mod sampler;
mod state;
mod types;
mod ws;

use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::str::FromStr;

mod tls;

use crate::sampler::{spawn_disks_sampler, spawn_process_sampler, spawn_sampler};
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

// (tests moved to end of file to satisfy clippy::items_after_test_module)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let state = AppState::new();

    // Start background sampler (adjust cadence as needed)
    // 500ms fast metrics
    let _h_fast = spawn_sampler(state.clone(), std::time::Duration::from_millis(500));
    // 2s processes (top 50)
    let _h_procs = spawn_process_sampler(state.clone(), std::time::Duration::from_secs(2), 50);
    // 5s disks
    let _h_disks = spawn_disks_sampler(state.clone(), std::time::Duration::from_secs(5));

    // Web app: route /ws to the websocket handler
    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
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

#[cfg(test)]
mod tests_cli_agent {
    // Local helper for testing port parsing
    fn parse_port<I: IntoIterator<Item = String>>(args: I, default_port: u16) -> u16 {
        let mut it = args.into_iter();
        let _ = it.next(); // prog
        let mut long: Option<String> = None;
        let mut short: Option<String> = None;
        while let Some(a) = it.next() {
            match a.as_str() {
                "--port" => long = it.next(),
                "-p" => short = it.next(),
                _ if a.starts_with("--port=") => {
                    if let Some((_, v)) = a.split_once('=') {
                        long = Some(v.to_string());
                    }
                }
                _ => {}
            }
        }
        long.or(short)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default_port)
    }

    #[test]
    fn port_long_short_and_assign() {
        assert_eq!(
            parse_port(vec!["agent".into(), "--port".into(), "9001".into()], 8443),
            9001
        );
        assert_eq!(
            parse_port(vec!["agent".into(), "-p".into(), "9002".into()], 8443),
            9002
        );
        assert_eq!(
            parse_port(vec!["agent".into(), "--port=9003".into()], 8443),
            9003
        );
        assert_eq!(parse_port(vec!["agent".into()], 8443), 8443);
    }
}
