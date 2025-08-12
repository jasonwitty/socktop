//! WebSocket upgrade and per-connection handler (request-driven).

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{Query, State, WebSocketUpgrade},
    response::Response,
};
use flate2::{write::GzEncoder, Compression};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::io::Write;

use crate::metrics::{collect_disks, collect_fast_metrics, collect_processes_top_k};
use crate::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    // optional auth
    if let Some(expected) = state.auth_token.as_ref() {
        if q.get("token") != Some(expected) {
            return ws.on_upgrade(|socket| async move {
                let _ = socket.close().await;
            });
        }
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    state
        .client_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(ref text) if text == "get_metrics" => {
                let m = collect_fast_metrics(&state).await;
                let _ = send_json(&mut socket, &m).await;
            }
            Message::Text(ref text) if text == "get_disks" => {
                let d = collect_disks(&state).await;
                let _ = send_json(&mut socket, &d).await;
            }
            Message::Text(ref text) if text == "get_processes" => {
                let p = collect_processes_top_k(&state, 50).await;
                let _ = send_json(&mut socket, &p).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    state
        .client_count
        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

// Small, cheap gzip for larger payloads; send text for small.
async fn send_json<T: serde::Serialize>(ws: &mut WebSocket, value: &T) -> Result<(), axum::Error> {
    let json = serde_json::to_string(value).expect("serialize");
    if json.len() <= 768 {
        return ws.send(Message::Text(json)).await;
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(json.as_bytes()).ok();
    let bin = enc.finish().unwrap_or_else(|_| json.into_bytes());
    ws.send(Message::Binary(bin)).await
}
