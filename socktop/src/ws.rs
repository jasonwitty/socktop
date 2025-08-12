//! Minimal WebSocket client helpers for requesting metrics from the agent.

use flate2::bufread::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use std::io::Read;
use tokio::net::TcpStream;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::types::{DiskInfo, Metrics, ProcessesPayload};

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
        Some(Ok(Message::Binary(b))) => {
            gunzip_to_string(&b).and_then(|s| serde_json::from_str::<Metrics>(&s).ok())
        }
        Some(Ok(Message::Text(json))) => serde_json::from_str::<Metrics>(&json).ok(),
        _ => None,
    }
}

// Decompress a gzip-compressed binary frame into a String.
fn gunzip_to_string(bytes: &[u8]) -> Option<String> {
    let mut dec = GzDecoder::new(bytes);
    let mut out = String::new();
    dec.read_to_string(&mut out).ok()?;
    Some(out)
}

// Suppress dead_code until these are wired into the app
#[allow(dead_code)]
pub enum Payload {
    Metrics(Metrics),
    Disks(Vec<DiskInfo>),
    Processes(ProcessesPayload),
}

#[allow(dead_code)]
fn parse_any_payload(json: &str) -> Result<Payload, serde_json::Error> {
    if let Ok(m) = serde_json::from_str::<Metrics>(json) {
        return Ok(Payload::Metrics(m));
    }
    if let Ok(d) = serde_json::from_str::<Vec<DiskInfo>>(json) {
        return Ok(Payload::Disks(d));
    }
    if let Ok(p) = serde_json::from_str::<ProcessesPayload>(json) {
        return Ok(Payload::Processes(p));
    }
    Err(serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "unknown payload",
    )))
}

// Send a "get_disks" request and await a JSON Vec<DiskInfo>
pub async fn request_disks(ws: &mut WsStream) -> Option<Vec<DiskInfo>> {
    if ws.send(Message::Text("get_disks".into())).await.is_err() {
        return None;
    }
    match ws.next().await {
        Some(Ok(Message::Binary(b))) => {
            gunzip_to_string(&b).and_then(|s| serde_json::from_str::<Vec<DiskInfo>>(&s).ok())
        }
        Some(Ok(Message::Text(json))) => serde_json::from_str::<Vec<DiskInfo>>(&json).ok(),
        _ => None,
    }
}

// Send a "get_processes" request and await a JSON ProcessesPayload
pub async fn request_processes(ws: &mut WsStream) -> Option<ProcessesPayload> {
    if ws
        .send(Message::Text("get_processes".into()))
        .await
        .is_err()
    {
        return None;
    }
    match ws.next().await {
        Some(Ok(Message::Binary(b))) => {
            gunzip_to_string(&b).and_then(|s| serde_json::from_str::<ProcessesPayload>(&s).ok())
        }
        Some(Ok(Message::Text(json))) => serde_json::from_str::<ProcessesPayload>(&json).ok(),
        _ => None,
    }
}

#[allow(dead_code)]
pub async fn start_ws_polling(mut ws: WsStream) {
    let mut t_fast = interval(Duration::from_millis(500));
    let mut t_procs = interval(Duration::from_secs(2));
    let mut t_disks = interval(Duration::from_secs(5));

    let _ = ws.send(Message::Text("get_metrics".into())).await;
    let _ = ws.send(Message::Text("get_processes".into())).await;
    let _ = ws.send(Message::Text("get_disks".into())).await;

    loop {
        tokio::select! {
            _ = t_fast.tick() => {
                let _ = ws.send(Message::Text("get_metrics".into())).await;
            }
            _ = t_procs.tick() => {
                let _ = ws.send(Message::Text("get_processes".into())).await;
            }
            _ = t_disks.tick() => {
                let _ = ws.send(Message::Text("get_disks".into())).await;
            }
            maybe = ws.next() => {
                let Some(result) = maybe else { break; };
                let Ok(msg) = result else { break; };
                match msg {
                    Message::Binary(b) => {
                        if let Some(json) = gunzip_to_string(&b) {
                            if let Ok(payload) = parse_any_payload(&json) {
                                match payload {
                                    Payload::Metrics(_m) => {
                                        // update your app state with fast metrics
                                    }
                                    Payload::Disks(_d) => {
                                        // update your app state with disks
                                    }
                                    Payload::Processes(_p) => {
                                        // update your app state with processes
                                    }
                                }
                            }
                        }
                    }
                    Message::Text(s) => {
                        if let Ok(payload) = parse_any_payload(&s) {
                            match payload {
                                Payload::Metrics(_m) => {}
                                Payload::Disks(_d) => {}
                                Payload::Processes(_p) => {}
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
}
