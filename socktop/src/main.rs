//! Entry point for the socktop TUI. Parses args and runs the App.

mod app;
mod history;
mod types;
mod ui;
mod ws;

use app::App;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args();
    let prog = args.next().unwrap_or_else(|| "socktop".into());
    let url = match args.next() {
        Some(flag) if flag == "-h" || flag == "--help" => {
            println!("Usage: {prog} ws://HOST:PORT/ws");
            return Ok(());
        }
        Some(url) => url,
        None => {
            eprintln!("Usage: {prog} ws://HOST:PORT/ws");
            std::process::exit(1);
        }
    };

    let mut app = App::new();
    app.run(&url).await
}
