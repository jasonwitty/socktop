//! Minimal WebSocket client helpers for requesting metrics from the agent.

use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use std::io::{Cursor, Read};
use std::sync::OnceLock;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::types::{DiskInfo, Metrics, ProcessesPayload};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[inline]
fn debug_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("SOCKTOP_DEBUG")
            .map(|v| v != "0")
            .unwrap_or(false)
    })
}

fn log_msg(msg: &Message) {
    match msg {
        Message::Binary(b) => eprintln!("ws: Binary {} bytes", b.len()),
        Message::Text(s) => eprintln!("ws: Text {} bytes", s.len()),
        Message::Close(_) => eprintln!("ws: Close"),
        _ => eprintln!("ws: Other frame"),
    }
}

// Connect to the agent and return the WS stream
pub async fn connect(url: &str) -> Result<WsStream, Box<dyn std::error::Error>> {
    if debug_on() {
        eprintln!("ws: connecting to {url}");
    }
    let (ws, _) = connect_async(url).await?;
    if debug_on() {
        eprintln!("ws: connected");
    }
    Ok(ws)
}

// Decompress a gzip-compressed binary frame into a String.
fn gunzip_to_string(bytes: &[u8]) -> Option<String> {
    let cursor = Cursor::new(bytes);
    let mut dec = GzDecoder::new(cursor);
    let mut out = String::new();
    dec.read_to_string(&mut out).ok()?;
    if debug_on() {
        eprintln!("ws: gunzip decoded {} bytes", out.len());
    }
    Some(out)
}

fn message_to_json(msg: &Message) -> Option<String> {
    match msg {
        Message::Binary(b) => {
            if debug_on() {
                eprintln!("ws: <- Binary frame {} bytes", b.len());
            }
            if let Some(s) = gunzip_to_string(b) {
                return Some(s);
            }
            // Fallback: try interpreting as UTF-8 JSON in a binary frame
            String::from_utf8(b.clone()).ok()
        }
        Message::Text(s) => {
            if debug_on() {
                eprintln!("ws: <- Text frame {} bytes", s.len());
            }
            Some(s.clone())
        }
        _ => None,
    }
}

// Suppress dead_code until these are wired into the app
#[allow(dead_code)]
pub enum Payload {
    Metrics(Metrics),
    Disks(Vec<DiskInfo>),
    Processes(ProcessesPayload),
}

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

// Send a "get_metrics" request and await a single JSON reply
pub async fn request_metrics(ws: &mut WsStream) -> Option<Metrics> {
    if debug_on() {
        eprintln!("ws: -> get_metrics");
    }
    if ws.send(Message::Text("get_metrics".into())).await.is_err() {
        return None;
    }
    // Drain a few messages until we find Metrics (handle out-of-order replies)
    for _ in 0..8 {
        match timeout(Duration::from_millis(800), ws.next()).await {
            Ok(Some(Ok(msg))) => {
                if debug_on() {
                    log_msg(&msg);
                }
                if let Some(json) = message_to_json(&msg) {
                    match parse_any_payload(&json) {
                        Ok(Payload::Metrics(m)) => return Some(m),
                        Ok(Payload::Disks(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Disks while waiting for Metrics");
                            }
                        }
                        Ok(Payload::Processes(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Processes while waiting for Metrics");
                            }
                        }
                        Err(_e) => {
                            if debug_on() {
                                eprintln!(
                                    "ws: unknown payload while waiting for Metrics (len={})",
                                    json.len()
                                );
                            }
                        }
                    }
                } else if debug_on() {
                    eprintln!("ws: non-json frame while waiting for Metrics");
                }
            }
            Ok(Some(Err(_e))) => continue,
            Ok(None) => return None,
            Err(_elapsed) => continue,
        }
    }
    None
}

// Send a "get_disks" request and await a JSON Vec<DiskInfo>
pub async fn request_disks(ws: &mut WsStream) -> Option<Vec<DiskInfo>> {
    if debug_on() {
        eprintln!("ws: -> get_disks");
    }
    if ws.send(Message::Text("get_disks".into())).await.is_err() {
        return None;
    }
    for _ in 0..8 {
        match timeout(Duration::from_millis(800), ws.next()).await {
            Ok(Some(Ok(msg))) => {
                if debug_on() {
                    log_msg(&msg);
                }
                if let Some(json) = message_to_json(&msg) {
                    match parse_any_payload(&json) {
                        Ok(Payload::Disks(d)) => return Some(d),
                        Ok(Payload::Metrics(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Metrics while waiting for Disks");
                            }
                        }
                        Ok(Payload::Processes(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Processes while waiting for Disks");
                            }
                        }
                        Err(_e) => {
                            if debug_on() {
                                eprintln!(
                                    "ws: unknown payload while waiting for Disks (len={})",
                                    json.len()
                                );
                            }
                        }
                    }
                } else if debug_on() {
                    eprintln!("ws: non-json frame while waiting for Disks");
                }
            }
            Ok(Some(Err(_e))) => continue,
            Ok(None) => return None,
            Err(_elapsed) => continue,
        }
    }
    None
}

// Send a "get_processes" request and await a JSON ProcessesPayload
pub async fn request_processes(ws: &mut WsStream) -> Option<ProcessesPayload> {
    if debug_on() {
        eprintln!("ws: -> get_processes");
    }
    if ws
        .send(Message::Text("get_processes".into()))
        .await
        .is_err()
    {
        return None;
    }
    for _ in 0..16 {
        // allow a few more cycles due to gzip size
        match timeout(Duration::from_millis(1200), ws.next()).await {
            Ok(Some(Ok(msg))) => {
                if debug_on() {
                    log_msg(&msg);
                }
                if let Some(json) = message_to_json(&msg) {
                    match parse_any_payload(&json) {
                        Ok(Payload::Processes(p)) => return Some(p),
                        Ok(Payload::Metrics(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Metrics while waiting for Processes");
                            }
                        }
                        Ok(Payload::Disks(_)) => {
                            if debug_on() {
                                eprintln!("ws: got Disks while waiting for Processes");
                            }
                        }
                        Err(_e) => {
                            if debug_on() {
                                eprintln!(
                                    "ws: unknown payload while waiting for Processes (len={})",
                                    json.len()
                                );
                            }
                        }
                    }
                } else if debug_on() {
                    eprintln!("ws: non-json frame while waiting for Processes");
                }
            }
            Ok(Some(Err(_e))) => continue,
            Ok(None) => return None,
            Err(_elapsed) => continue,
        }
    }
    None
}
