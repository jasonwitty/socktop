//! Minimal WebSocket client helpers for requesting metrics from the agent.

use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::types::Metrics;

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

// Connect to the agent and return the WS stream
pub async fn connect(url: &str) -> Result<WsStream, Box<dyn std::error::Error>> {
    let (ws, _) = connect_async(url).await?;
    Ok(ws)
}

// Send a "get_metrics" request and await a single JSON reply
pub async fn request_metrics(ws: &mut WsStream) -> Option<Metrics> {
    if ws.send(Message::Text("get_metrics".into())).await.is_err() {
        return None;
    }
    match ws.next().await {
        Some(Ok(Message::Text(json))) => serde_json::from_str::<Metrics>(&json).ok(),
        _ => None,
    }
}

// Re-export SinkExt/StreamExt for call sites
use futures_util::{SinkExt, StreamExt};