//! Minimal WebSocket client helpers for requesting metrics from the agent.

use flate2::bufread::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::Item;
use std::io::{Cursor, Read};
use std::sync::OnceLock;
use std::{fs::File, io::BufReader, sync::Arc};
use tokio::net::TcpStream;
use tokio::time::{interval, timeout, Duration};
use tokio_tungstenite::{
    connect_async, connect_async_tls_with_config, tungstenite::client::IntoClientRequest,
    tungstenite::Message, Connector, MaybeTlsStream, WebSocketStream,
};
use url::Url;

use crate::types::{DiskInfo, Metrics, ProcessesPayload};

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

// Connect to the agent and return the WS stream
pub async fn connect(
    url: &str,
    tls_ca: Option<&str>,
) -> Result<WsStream, Box<dyn std::error::Error>> {
    let mut u = Url::parse(url)?;
    if let Some(ca_path) = tls_ca {
        if u.scheme() == "ws" {
            let _ = u.set_scheme("wss");
        }
        return connect_with_ca(u.as_str(), ca_path).await;
    }
    let (ws, _) = connect_async(u.as_str()).await?;
    Ok(ws)
}

async fn connect_with_ca(url: &str, ca_path: &str) -> Result<WsStream, Box<dyn std::error::Error>> {
    let mut root = RootCertStore::empty();
    let mut reader = BufReader::new(File::open(ca_path)?);
    let mut der_certs = Vec::new();
    while let Ok(Some(item)) = rustls_pemfile::read_one(&mut reader) {
        if let Item::X509Certificate(der) = item {
            der_certs.push(der);
        }
    }
    root.add_parsable_certificates(der_certs);

    let cfg = ClientConfig::builder()
        .with_root_certificates(root)
        .with_no_client_auth();
    let cfg = Arc::new(cfg);

    let req = url.into_client_request()?;
    let (ws, _) =
        connect_async_tls_with_config(req, None, true, Some(Connector::Rustls(cfg))).await?;
    Ok(ws)
}

#[inline]
fn debug_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("SOCKTOP_DEBUG")
            .map(|v| v != "0")
            .unwrap_or(false)
    })
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
