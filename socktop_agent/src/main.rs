use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::stream::StreamExt;
use serde::Serialize;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, ProcessRefreshKind, RefreshKind,
    System,
};
use tokio::sync::Mutex;

// ---------- Data types sent to the client ----------

#[derive(Debug, Serialize, Clone)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    mem_bytes: u64,
}

#[derive(Debug, Serialize, Clone)]
struct DiskInfo {
    name: String,
    total: u64,
    available: u64,
}

#[derive(Debug, Serialize, Clone)]
struct NetworkInfo {
    name: String,
    // cumulative totals since the agent started (client should diff to get rates)
    received: u64,
    transmitted: u64,
}

#[derive(Debug, Serialize, Clone)]
struct Metrics {
    cpu_total: f32,
    cpu_per_core: Vec<f32>,
    mem_total: u64,
    mem_used: u64,
    swap_total: u64,
    swap_used: u64,
    process_count: usize,
    hostname: String,
    cpu_temp_c: Option<f32>,
    disks: Vec<DiskInfo>,
    networks: Vec<NetworkInfo>,
    top_processes: Vec<ProcessInfo>,
}

// ---------- Shared state ----------

type SharedSystem = Arc<Mutex<System>>;
type SharedNetworks = Arc<Mutex<Networks>>;
type SharedTotals = Arc<Mutex<HashMap<String, (u64, u64)>>>; // iface -> (rx_total, tx_total)

#[derive(Clone)]
struct AppState {
    sys: SharedSystem,
    nets: SharedNetworks,
    net_totals: SharedTotals,
}

#[tokio::main]
async fn main() {
    // sysinfo 0.36: build specifics
    let refresh_kind = RefreshKind::nothing()
        .with_cpu(CpuRefreshKind::everything())
        .with_memory(MemoryRefreshKind::everything())
        .with_processes(ProcessRefreshKind::everything());

    let mut sys = System::new_with_specifics(refresh_kind);
    sys.refresh_all();

    // Keep Networks alive across requests so received()/transmitted() deltas work
    let mut nets = Networks::new();
    nets.refresh(true);

    let shared = Arc::new(Mutex::new(sys));
    let shared_nets = Arc::new(Mutex::new(nets));
    let net_totals: SharedTotals = Arc::new(Mutex::new(HashMap::new()));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(AppState {
            sys: shared,
            nets: shared_nets,
            net_totals,
        });

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Remote agent running at http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    while let Some(Ok(msg)) = socket.next().await {
        if let Message::Text(text) = msg {
            if text == "get_metrics" {
                let metrics = collect_metrics(&state).await;
                let json = serde_json::to_string(&metrics).unwrap();
                let _ = socket.send(Message::Text(json)).await;
            }
        }
    }
}

// ---------- Metrics collection ----------

async fn collect_metrics(state: &AppState) -> Metrics {
    // System (CPU/mem/proc)
    let mut sys = state.sys.lock().await;
    sys.refresh_all();

    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());

    // Temps via Components (separate handle in 0.36)
    let mut components = Components::new();
    components.refresh(true);
    let cpu_temp_c = best_cpu_temp(&components);

    // Disks (separate handle in 0.36)
    let mut disks_struct = Disks::new();
    disks_struct.refresh(true);
    // Filter anything with available == 0 (e.g., overlay)
    let disks: Vec<DiskInfo> = disks_struct
        .list()
        .iter()
        .filter(|d| d.available_space() > 0)
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().to_string(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();

    // Networks: use a persistent Networks + rolling totals
    let mut nets = state.nets.lock().await;
    nets.refresh(true);

    let mut totals = state.net_totals.lock().await;
    let mut networks: Vec<NetworkInfo> = Vec::new();

    for (name, data) in nets.iter() {
        // sysinfo 0.36: data.received()/transmitted() are deltas since *last* refresh
        let delta_rx = data.received();
        let delta_tx = data.transmitted();

        let entry = totals.entry(name.clone()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(delta_rx);
        entry.1 = entry.1.saturating_add(delta_tx);

        networks.push(NetworkInfo {
            name: name.clone(),
            received: entry.0,
            transmitted: entry.1,
        });
    }

    // get number of cpu cores
    let n_cpus = sys.cpus().len().max(1) as f32;

    // Top processes: include PID and memory, top 20 by CPU
    let mut top_processes: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_usage: (p.cpu_usage() / n_cpus).min(100.0),
            mem_bytes: p.memory(), // sysinfo 0.36: bytes
        })
        .collect();
    top_processes.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap());
    top_processes.truncate(20);

    Metrics {
        cpu_total: sys.global_cpu_usage(),
        cpu_per_core: sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
        mem_total: sys.total_memory(),
        mem_used: sys.used_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        process_count: sys.processes().len(),
        hostname,
        cpu_temp_c,
        disks,
        networks,
        top_processes,
    }
}

fn best_cpu_temp(components: &Components) -> Option<f32> {
    components
        .iter()
        .filter(|c| {
            let label = c.label().to_lowercase();
            label.contains("cpu") || label.contains("package") || label.contains("tctl") || label.contains("tdie")
        })
        .filter_map(|c| c.temperature())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}
