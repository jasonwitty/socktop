//! WebSocket upgrade and per-connection handler (request-driven).

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{Query, State, WebSocketUpgrade},
    response::Response,
};
use flate2::{Compression, write::GzEncoder};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::io::Write;
use std::time::Duration as StdDuration;
use tokio::sync::Mutex;

use crate::metrics::{
    collect_disks, collect_fast_metrics, collect_processes_all, disks_ttl_ms, metrics_ttl_ms,
    processes_ttl_ms,
};
use crate::proto::pb;
use crate::state::{AppState, CacheEntry};

// Payloads at or below this many bytes are sent as-is; larger ones are gzipped.
const COMPRESSION_THRESHOLD: usize = 768;

/// The frame for a JSON payload: a Text frame when it is small enough that gzip
/// costs more than it saves, a gzipped Binary frame otherwise. The client tells
/// them apart by frame type and the gzip magic, so the rule has to stay.
fn json_frame<T: serde::Serialize>(value: &T) -> Message {
    let json = serde_json::to_string(value).expect("serialize");
    if json.len() <= COMPRESSION_THRESHOLD {
        return Message::Text(json);
    }
    // A new encoder per frame so the gzip headers are complete.
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    match enc.write_all(json.as_bytes()).and_then(|_| enc.finish()) {
        Ok(bin) => Message::Binary(bin),
        Err(_) => Message::Binary(json.into_bytes()),
    }
}

/// The frame for the process list: protobuf rows, gzipped under the same size
/// rule as JSON. Encoding can only fail if the buffer cannot grow, and the old
/// code closed the socket in that case; returning the Close frame here does the
/// same thing, since `respond_frame` sends whatever this hands back.
fn processes_frame(payload: crate::types::ProcessesPayload) -> Message {
    let msg = pb::Processes {
        process_count: payload.process_count as u64,
        rows: payload
            .top_processes
            .into_iter()
            .map(|p| pb::Process {
                pid: p.pid,
                name: p.name,
                cpu_usage: p.cpu_usage,
                mem_bytes: p.mem_bytes,
            })
            .collect(),
    };
    let mut buf = Vec::with_capacity(8 * 1024);
    if prost::Message::encode(&msg, &mut buf).is_err() {
        return Message::Close(None);
    }
    if buf.len() <= COMPRESSION_THRESHOLD {
        return Message::Binary(buf);
    }
    let mut encoder = GzEncoder::new(Vec::with_capacity(buf.len()), Compression::fast());
    match encoder.write_all(&buf).and_then(|_| encoder.finish()) {
        Ok(compressed) => Message::Binary(compressed),
        Err(_) => Message::Binary(buf),
    }
}

/// One home for "answer this request from a TTL cache of the encoded frame":
/// collect and encode on a miss, hand out a `Message` clone on a hit.
///
/// The caches used to hold decoded payloads and the request path re-encoded them
/// every time, so a hit cost the same as a miss: for `get_processes` that was a
/// clone of the payload (one String per process), a fresh 8 KB encode buffer,
/// prost, and gzip — 0.17 ms of agent CPU per hit on a desktop, around 1 ms on a
/// Pi, which is how ten unauthenticated clients could pin a core. The frame is
/// now decided once per TTL and a hit is a refcount bump and a write.
async fn respond_frame<T, Fut>(
    socket: &mut WebSocket,
    cache: &Mutex<CacheEntry<Message>>,
    ttl: StdDuration,
    collect: impl FnOnce() -> Fut,
    encode: impl FnOnce(T) -> Message,
) where
    Fut: std::future::Future<Output = T>,
{
    {
        let cache = cache.lock().await;
        if cache.is_fresh(ttl)
            && let Some(frame) = cache.get()
        {
            let _ = socket.send(frame.clone()).await;
            return;
        }
    }
    let frame = encode(collect().await);
    cache.lock().await.set(frame.clone());
    let _ = socket.send(frame).await;
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    // optional auth
    if let Some(expected) = state.auth_token.as_ref()
        && q.get("token") != Some(expected)
    {
        return ws.on_upgrade(|socket| async move {
            let _ = socket.close().await;
        });
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-PID cache limits: entries older than MAX_AGE are swept on every
/// insert and the map is capped at MAX_ENTRIES (oldest evicted first), so a
/// client walking PIDs cannot grow agent memory without bound.
const PER_PID_CACHE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);
const PER_PID_CACHE_MAX_ENTRIES: usize = 64;

/// Serve a per-PID request from a TTL cache, collecting on miss. One home
/// for the logic that get_process_metrics and get_journal_entries used to
/// duplicate ~50 lines apiece.
async fn respond_per_pid_cached<T, Fut>(
    socket: &mut WebSocket,
    cache: &Mutex<HashMap<u32, crate::state::CacheEntry<T>>>,
    pid: u32,
    ttl: std::time::Duration,
    request_name: &str,
    collect: impl FnOnce() -> Fut,
) where
    T: serde::Serialize + Clone,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    {
        let cache = cache.lock().await;
        if let Some(entry) = cache.get(&pid)
            && entry.is_fresh(ttl)
            && let Some(v) = entry.get()
        {
            let _ = send_json(socket, v).await;
            return;
        }
    }
    match collect().await {
        Ok(resp) => {
            {
                let mut cache = cache.lock().await;
                cache.retain(|_, e| e.at.is_some_and(|t| t.elapsed() < PER_PID_CACHE_MAX_AGE));
                while cache.len() >= PER_PID_CACHE_MAX_ENTRIES {
                    let oldest = cache.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| *k);
                    match oldest {
                        Some(k) => cache.remove(&k),
                        None => break,
                    };
                }
                cache
                    .entry(pid)
                    .or_insert_with(crate::state::CacheEntry::new)
                    .set(resp.clone());
            }
            let _ = send_json(socket, &resp).await;
        }
        Err(err) => {
            let error_response = serde_json::json!({
                "error": err,
                "request": request_name,
                "pid": pid
            });
            let _ = send_json(socket, &error_response).await;
        }
    }
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    state
        .client_count
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(ref text) if text == "get_metrics" => {
                respond_frame(
                    &mut socket,
                    &state.cache_metrics,
                    StdDuration::from_millis(metrics_ttl_ms()),
                    || collect_fast_metrics(&state),
                    |m| json_frame(&m),
                )
                .await;
            }
            Message::Text(ref text) if text == "get_disks" => {
                respond_frame(
                    &mut socket,
                    &state.cache_disks,
                    StdDuration::from_millis(disks_ttl_ms()),
                    || collect_disks(&state),
                    |d| json_frame(&d),
                )
                .await;
            }
            Message::Text(ref text) if text == "get_processes" => {
                respond_frame(
                    &mut socket,
                    &state.cache_processes,
                    StdDuration::from_millis(processes_ttl_ms()),
                    || collect_processes_all(&state),
                    processes_frame,
                )
                .await;
            }
            Message::Text(ref text) if text.starts_with("get_process_metrics:") => {
                if let Some(pid_str) = text.strip_prefix("get_process_metrics:")
                    && let Ok(pid) = pid_str.parse::<u32>()
                {
                    respond_per_pid_cached(
                        &mut socket,
                        &state.cache_process_metrics,
                        pid,
                        std::time::Duration::from_millis(250),
                        "get_process_metrics",
                        || crate::metrics::collect_process_metrics(pid, &state),
                    )
                    .await;
                }
            }
            Message::Text(ref text) if text.starts_with("get_journal_entries:") => {
                if let Some(pid_str) = text.strip_prefix("get_journal_entries:")
                    && let Ok(pid) = pid_str.parse::<u32>()
                {
                    respond_per_pid_cached(
                        &mut socket,
                        &state.cache_journal_entries,
                        pid,
                        std::time::Duration::from_secs(1),
                        "get_journal_entries",
                        || crate::metrics::collect_journal_entries(pid),
                    )
                    .await;
                }
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
    ws.send(json_frame(value)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as ProstMessage;
    use std::io::Read;
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

        // The request path now builds one encoded frame per TTL, so the test goes
        // through `processes_frame` and has to decode whatever it produces: a
        // gzipped Binary frame for a real process list, plain Binary if the list
        // happens to be under the compression threshold.
        let raw = match processes_frame(processes) {
            Message::Binary(b) if b.starts_with(&[0x1f, 0x8b]) => {
                let mut out = Vec::new();
                flate2::read::GzDecoder::new(b.as_slice())
                    .read_to_end(&mut out)
                    .expect("gunzip the process frame");
                out
            }
            Message::Binary(b) => b,
            other => panic!("expected a binary process frame, got {other:?}"),
        };

        let pb = pb::Processes::decode(raw.as_slice()).expect("Failed to decode protobuf");
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
