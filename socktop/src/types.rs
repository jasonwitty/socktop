//! Types that mirror the agent's JSON schema.

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Disk {
    pub name: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Network {
    // cumulative totals; client diffs to compute rates
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GpuMetrics {
    pub name: String,
    pub utilization_gpu_pct: u32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
  //  pub vendor: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
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
    pub disks: Vec<Disk>,
    pub networks: Vec<Network>,
    pub top_processes: Vec<ProcessInfo>,
    pub gpus: Option<Vec<GpuMetrics>>,
}
