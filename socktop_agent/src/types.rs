//! Data types sent to the client over WebSocket.
//! Keep this module minimal and stable — it defines the wire format.

use crate::gpu::GpuMetrics;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub total: u64,
    pub available: u64,
    pub temperature: Option<f32>,
    pub is_partition: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkInfo {
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    /// Epoch ms when this snapshot was actually collected. The agent serves
    /// TTL-cached snapshots, so the client needs the AGENT's sample time to
    /// compute rates — measuring against client receive time turned cache
    /// hits into a 0-then-2x sawtooth in the network graphs.
    pub sampled_at_ms: u64,
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
    pub gpus: Option<Vec<GpuMetrics>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessesPayload {
    pub process_count: usize,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadInfo {
    pub tid: u32,             // Thread ID
    pub name: String,         // Thread name (from /proc/{pid}/task/{tid}/comm)
    pub cpu_time_user: u64,   // User CPU time in microseconds
    pub cpu_time_system: u64, // System CPU time in microseconds
    pub status: String,       // Thread status (Running, Sleeping, etc.)
}

#[derive(Debug, Clone, Serialize)]
pub struct DetailedProcessInfo {
    pub pid: u32,
    pub name: String,
    pub command: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
    pub virtual_mem_bytes: u64,
    pub shared_mem_bytes: Option<u64>,
    pub thread_count: u32,
    pub fd_count: Option<u32>,
    pub status: String,
    pub parent_pid: Option<u32>,
    pub user_id: u32,
    pub group_id: u32,
    pub start_time: u64,      // Unix timestamp
    pub cpu_time_user: u64,   // Microseconds
    pub cpu_time_system: u64, // Microseconds
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub working_directory: Option<String>,
    pub executable_path: Option<String>,
    pub child_processes: Vec<DetailedProcessInfo>,
    pub threads: Vec<ThreadInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessMetricsResponse {
    pub process: DetailedProcessInfo,
    pub cached_at: u64, // Unix timestamp when this data was cached
}

#[derive(Debug, Clone, Serialize)]
pub struct JournalEntry {
    pub timestamp: String, // RFC 3339 UTC, for display
    pub timestamp_us: u64, // epoch microseconds, for sorting/formatting
    pub priority: LogLevel,
    pub message: String,
    pub unit: Option<String>, // systemd unit name
    pub pid: Option<u32>,
    pub comm: Option<String>, // process command name
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub enum LogLevel {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

#[derive(Debug, Clone, Serialize)]
pub struct JournalResponse {
    pub entries: Vec<JournalEntry>,
    pub total_count: u32,
    pub truncated: bool,
    /// journalctl's own explanation when the result is empty because of
    /// journal ACCESS (not absence of logs) — e.g. a user-run agent asking
    /// about a system service. None when entries exist or nothing to say.
    pub notice: Option<String>,
    pub cached_at: u64, // Unix timestamp when this data was cached
}
