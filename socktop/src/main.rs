//! Entry point for the socktop TUI. Parses args and runs the App.

mod app;
mod history;
mod types;
mod ui;
mod ws;

use std::env;
use app::App;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} ws://HOST:PORT/ws", args[0]);
        std::process::exit(1);
    }
    let url = args[1].clone();

    let mut app = App::new();
    app.run(&url).await
}