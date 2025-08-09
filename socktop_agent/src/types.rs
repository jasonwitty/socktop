//! Data types sent to the client over WebSocket.
//! Keep this module minimal and stable — it defines the wire format.

use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct NetworkInfo {
    pub name: String,
    // cumulative totals since the agent started (client should diff to get rates)
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct Metrics {
    pub cpu_total: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub process_count: usize,
    pub hostname: String,
    pub cpu_temp_c: Option<f32>,
    pub disks: Vec<DiskInfo>,
    pub networks: Vec<NetworkInfo>,
    pub top_processes: Vec<ProcessInfo>,
}
