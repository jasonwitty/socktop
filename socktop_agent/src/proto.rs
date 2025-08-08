use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metrics {
    pub ts_unix_ms: i64,
    pub host: String,
    pub uptime_secs: u64,
    pub cpu_overall: f32,
    pub cpu_per_core: Vec<f32>,
    pub load_avg: (f64, f64, f64),
    pub mem_total_mb: u64,
    pub mem_used_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub net_aggregate: NetTotals,
    pub top_processes: Vec<Proc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetTotals {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proc {
    pub pid: i32,
    pub name: String,
    pub cpu: f32,
    pub mem_mb: u64,
    pub status: String,
}