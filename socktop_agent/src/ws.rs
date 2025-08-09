//! WebSocket upgrade and per-connection handler. Serves cached JSON quickly.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures_util::stream::StreamExt;

use crate::metrics::collect_metrics;
use crate::state::AppState;

use std::collections::HashMap;
use std::sync::atomic::Ordering;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Some(expected) = state.auth_token.as_ref() {
        match q.get("token") {
            Some(t) if t == expected => {}
            _ => return StatusCode::UNAUTHORIZED.into_response(),
        }
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Bump client count on connect and wake the sampler.
    state.client_count.fetch_add(1, Ordering::Relaxed);
    state.wake_sampler.notify_waiters();

    // Ensure we decrement on disconnect (drop).
    struct ClientGuard(AppState);
    impl Drop for ClientGuard {
        fn drop(&mut self) {
            self.0.client_count.fetch_sub(1, Ordering::Relaxed);
            self.0.wake_sampler.notify_waiters();
        }
    }
    let _guard = ClientGuard(state.clone());

    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(text) if text == "get_metrics" => {
                // Serve the cached JSON quickly; if empty (cold start), collect once.
                let cached = state.last_json.read().await.clone();
                if !cached.is_empty() {
                    let _ = socket.send(Message::Text(cached)).await;
                } else {
                    let metrics = collect_metrics(&state).await;
                    if let Ok(js) = serde_json::to_string(&metrics) {
                        let _ = socket.send(Message::Text(js)).await;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
