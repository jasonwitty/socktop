//! WebSocket upgrade and per-connection handler (request-driven).

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{Query, State, WebSocketUpgrade},
    response::Response,
};
use flate2::{write::GzEncoder, Compression};
use futures_util::StreamExt;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::io::Write;
use tokio::sync::Mutex;

use crate::metrics::{collect_disks, collect_fast_metrics, collect_processes_all};
use crate::proto::pb;
use crate::state::AppState;

// Compression threshold based on typical payload size
const COMPRESSION_THRESHOLD: usize = 768;

// Reusable buffer for compression to avoid allocations
struct CompressionCache {
    processes_vec: Vec<pb::Process>,
}

impl CompressionCache {
    fn new() -> Self {
        Self {
            processes_vec: Vec::with_capacity(512), // Typical process count
        }
    }
}

static COMPRESSION_CACHE: OnceCell<Mutex<CompressionCache>> = OnceCell::new();

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
                let payload = collect_processes_all(&state).await;

                // Map to protobuf message
                // Get cached buffers
                let cache = COMPRESSION_CACHE.get_or_init(|| Mutex::new(CompressionCache::new()));
                let mut cache = cache.lock().await;

                // Reuse process vector to build the list
                cache.processes_vec.clear();
                cache
                    .processes_vec
                    .extend(payload.top_processes.into_iter().map(|p| pb::Process {
                        pid: p.pid,
                        name: p.name,
                        cpu_usage: p.cpu_usage,
                        mem_bytes: p.mem_bytes,
                    }));

                let pb = pb::Processes {
                    process_count: payload.process_count as u64,
                    rows: std::mem::take(&mut cache.processes_vec),
                };

                let mut buf = Vec::with_capacity(8 * 1024);
                if prost::Message::encode(&pb, &mut buf).is_err() {
                    let _ = socket.send(Message::Close(None)).await;
                } else {
                    // compress if large
                    if buf.len() <= COMPRESSION_THRESHOLD {
                        let _ = socket.send(Message::Binary(buf)).await;
                    } else {
                        // Create a new encoder for each message to ensure proper gzip headers
                        let mut encoder =
                            GzEncoder::new(Vec::with_capacity(buf.len()), Compression::fast());
                        match encoder.write_all(&buf).and_then(|_| encoder.finish()) {
                            Ok(compressed) => {
                                let _ = socket.send(Message::Binary(compressed)).await;
                            }
                            Err(_) => {
                                let _ = socket.send(Message::Binary(buf)).await;
                            }
                        }
                    }
                }
                drop(cache); // Explicit drop to release mutex early
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
    if json.len() <= COMPRESSION_THRESHOLD {
        return ws.send(Message::Text(json)).await;
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(json.as_bytes()).ok();
    let bin = enc.finish().unwrap_or_else(|_| json.into_bytes());
    ws.send(Message::Binary(bin)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as ProstMessage;
    use sysinfo::System;

    #[tokio::test]
    async fn test_process_list_not_empty() {
        // Initialize system data first to ensure we have processes
        let mut sys = System::new_all();
        sys.refresh_all();

        // Create state and put the refreshed system in it
        let state = AppState::new();
        {
            let mut sys_lock = state.sys.lock().await;
            *sys_lock = sys;
        }

        // Get processes directly using the collection function
        let processes = collect_processes_all(&state).await;

        // Convert to protobuf message format
        let cache = COMPRESSION_CACHE.get_or_init(|| Mutex::new(CompressionCache::new()));
        let mut cache = cache.lock().await;

        // Reuse process vector to build the list
        cache.processes_vec.clear();
        cache
            .processes_vec
            .extend(processes.top_processes.into_iter().map(|p| pb::Process {
                pid: p.pid,
                name: p.name,
                cpu_usage: p.cpu_usage,
                mem_bytes: p.mem_bytes,
            }));

        // Create the protobuf message
        let pb = pb::Processes {
            process_count: processes.process_count as u64,
            rows: cache.processes_vec.clone(),
        };

        // Test protobuf encoding/decoding
        let mut buf = Vec::new();
        prost::Message::encode(&pb, &mut buf).expect("Failed to encode protobuf");
        let decoded = pb::Processes::decode(buf.as_slice()).expect("Failed to decode protobuf");

        // Print debug info
        println!("Process count: {}", pb.process_count);
        println!("Process vector length: {}", pb.rows.len());
        println!("Encoded size: {} bytes", buf.len());
        println!("Decoded process count: {}", decoded.rows.len());

        // Print first few processes if available
        for (i, process) in pb.rows.iter().take(5).enumerate() {
            println!(
                "Process {}: {} (PID: {}) CPU: {:.1}% MEM: {} bytes",
                i + 1,
                process.name,
                process.pid,
                process.cpu_usage,
                process.mem_bytes
            );
        }

        // Validate
        assert!(!pb.rows.is_empty(), "Process list should not be empty");
        assert!(
            pb.process_count > 0,
            "Process count should be greater than 0"
        );
        assert_eq!(
            pb.process_count as usize,
            pb.rows.len(),
            "Process count mismatch with actual rows"
        );
    }
}
