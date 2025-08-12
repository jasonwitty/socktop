//! Types that mirror the agent's JSON schema.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkInfo {
    #[allow(dead_code)]
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GpuInfo {
    pub name: Option<String>,
    #[allow(dead_code)]
    pub vendor: Option<String>,

    // Accept both the new and legacy keys
    #[serde(
        default,
        alias = "utilization_gpu_pct",
        alias = "gpu_util_pct",
        alias = "gpu_utilization"
    )]
    pub utilization: Option<f32>,

    #[serde(default, alias = "mem_used_bytes", alias = "vram_used_bytes")]
    pub mem_used: Option<u64>,

    #[serde(default, alias = "mem_total_bytes", alias = "vram_total_bytes")]
    pub mem_total: Option<u64>,

    #[allow(dead_code)]
    #[serde(default, alias = "temp_c", alias = "temperature_c")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metrics {
    pub cpu_total: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub hostname: String,
    pub cpu_temp_c: Option<f32>,
    pub disks: Vec<DiskInfo>,
    pub networks: Vec<NetworkInfo>,
    pub top_processes: Vec<ProcessInfo>,
    pub gpus: Option<Vec<GpuInfo>>,
    // New: keep the last reported total process count
    #[serde(default)]
    pub process_count: Option<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessesPayload {
    pub process_count: usize,
    pub top_processes: Vec<ProcessInfo>,
}
