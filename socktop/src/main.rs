//! Entry point for the socktop TUI. Parses args and runs the App.

mod app;
mod history;
mod types;
mod ui;
mod ws;

use app::App;
use std::env;

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<(String, Option<String>), String> {
    let mut it = args.into_iter();
    let prog = it.next().unwrap_or_else(|| "socktop".into());
    let mut url: Option<String> = None;
    let mut tls_ca: Option<String> = None;

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                return Err(format!(
                    "Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] ws://HOST:PORT/ws"
                ));
            }
            "--tls-ca" | "-t" => {
                tls_ca = it.next();
            }
            _ if arg.starts_with("--tls-ca=") => {
                if let Some((_, v)) = arg.split_once('=') {
                    if !v.is_empty() {
                        tls_ca = Some(v.to_string());
                    }
                }
            }
            _ => {
                if url.is_none() {
                    url = Some(arg);
                } else {
                    return Err(format!(
                        "Unexpected argument. Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] ws://HOST:PORT/ws"
                    ));
                }
            }
        }
    }

    match url {
        Some(u) => Ok((u, tls_ca)),
        None => Err(format!(
            "Usage: {prog} [--tls-ca CERT_PEM|-t CERT_PEM] ws://HOST:PORT/ws"
        )),
    }
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reuse the same parsing logic for testability
    let (url, tls_ca) = match parse_args(env::args()) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return Ok(());
        }
    };

    let mut app = App::new();
    app.run(&url, tls_ca.as_deref()).await
}
