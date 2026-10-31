//! Types that represent data from the socktop agent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub total: u64,
    pub available: u64,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub is_partition: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkInfo {
    pub name: String,
    pub received: u64,
    pub transmitted: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GpuInfo {
    pub name: Option<String>,
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

    #[serde(default, alias = "temp_c", alias = "temperature_c")]
    pub temp: Option<f32>,

    /// Die temperature at which the card begins throttling, when the driver
    /// reports it. Static per card, so the client can colour `temp` against the
    /// card's own limit instead of a constant guessed for a different model.
    #[serde(default, alias = "gpu_temp_slowdown_c")]
    pub temp_slowdown_c: Option<f32>,

    /// VRAM temperature limit. NVML publishes the limit without a live reading,
    /// so this is context, not a gauge.
    #[serde(default)]
    pub temp_mem_max_c: Option<f32>,

    #[serde(default, alias = "power_usage_w")]
    pub power_w: Option<f32>,

    /// The power limit the card is currently held to, which is not necessarily
    /// its factory limit on a host that has been power-capped.
    #[serde(default)]
    pub power_limit_w: Option<f32>,

    #[serde(default, alias = "fan_speed_pct")]
    pub fan_pct: Option<u32>,

    /// Driver-side device index, stable for the life of the driver session.
    #[serde(default, alias = "index", alias = "device_index")]
    pub nvml_index: Option<u32>,

    /// Strongest available card identity; used to avoid listing the primary card
    /// twice when an agent reports it through both the display driver and NVML.
    #[serde(default)]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Metrics {
    /// Epoch ms when the agent actually collected this snapshot (agents may
    /// serve TTL-cached data). Absent on agents older than 1.60.
    #[serde(default)]
    pub sampled_at_ms: Option<u64>,
    pub cpu_total: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_total: u64,
    /// MemTotal - MemAvailable: what the memory gauge's percentage is based
    /// on. Reclaimable memory (clean file-backed pages, i.e. an mmap'd model
    /// file) is deliberately NOT included.
    pub mem_used: u64,
    /// MemFree. Absent (0) on agents that predate the field, and equal to
    /// `mem_available` on Windows, so clients must treat a zero value as "no
    /// cache segment available" rather than "no free memory".
    #[serde(default)]
    pub mem_free: u64,
    /// MemAvailable: MemFree plus reclaimable page/buffer cache. Subtracting
    /// `mem_free` gives the cache slice.
    #[serde(default)]
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub hostname: String,
    pub cpu_temp_c: Option<f32>,
    pub disks: Vec<DiskInfo>,
    pub networks: Vec<NetworkInfo>,
    pub top_processes: Vec<ProcessInfo>,
    pub gpus: Option<Vec<GpuInfo>>,
    /// Cards beyond the primary one, absent on agents that predate multi-GPU
    /// reporting (and on single-card hosts, which send nothing). The client
    /// recombines the two fields in `ui::gpu::gpu_set()`; the primary card is
    /// kept separate on the wire so a client that only ever reads `gpus` keeps
    /// showing exactly the card it always showed.
    #[serde(default, alias = "additional_gpus", alias = "extra_gpus")]
    pub gpus_extra: Option<Vec<GpuInfo>>,
    // New: keep the last reported total process count
    #[serde(default)]
    pub process_count: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessesPayload {
    pub process_count: usize,
    pub top_processes: Vec<ProcessInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThreadInfo {
    pub tid: u32,             // Thread ID
    pub name: String,         // Thread name (from /proc/{pid}/task/{tid}/comm)
    pub cpu_time_user: u64,   // User CPU time in microseconds
    pub cpu_time_system: u64, // System CPU time in microseconds
    pub status: String,       // Thread status (Running, Sleeping, etc.)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessMetricsResponse {
    pub process: DetailedProcessInfo,
    pub cached_at: u64, // Unix timestamp when this data was cached
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JournalEntry {
    pub timestamp: String, // ISO 8601 formatted timestamp
    pub priority: LogLevel,
    pub message: String,
    pub unit: Option<String>, // systemd unit name
    pub pid: Option<u32>,
    pub comm: Option<String>, // process command name
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JournalResponse {
    pub entries: Vec<JournalEntry>,
    pub total_count: u32,
    pub truncated: bool,
    /// Agent-side explanation for an empty result (journal access limits).
    /// Absent on agents older than 1.60.
    #[serde(default)]
    pub notice: Option<String>,
    pub cached_at: u64, // Unix timestamp when this data was cached
}

/// Request types that can be sent to the agent
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum AgentRequest {
    #[serde(rename = "metrics")]
    Metrics,
    #[serde(rename = "disks")]
    Disks,
    #[serde(rename = "processes")]
    Processes,
    #[serde(rename = "process_metrics")]
    ProcessMetrics { pid: u32 },
    #[serde(rename = "journal_entries")]
    JournalEntries { pid: u32 },
}

impl AgentRequest {
    /// Convert to the legacy string format used by the agent
    pub fn to_legacy_string(&self) -> String {
        match self {
            AgentRequest::Metrics => "get_metrics".to_string(),
            AgentRequest::Disks => "get_disks".to_string(),
            AgentRequest::Processes => "get_processes".to_string(),
            AgentRequest::ProcessMetrics { pid } => format!("get_process_metrics:{pid}"),
            AgentRequest::JournalEntries { pid } => format!("get_journal_entries:{pid}"),
        }
    }
}

/// Response types that can be received from the agent
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum AgentResponse {
    #[serde(rename = "metrics")]
    Metrics(Metrics),
    #[serde(rename = "disks")]
    Disks(Vec<DiskInfo>),
    #[serde(rename = "processes")]
    Processes(ProcessesPayload),
    #[serde(rename = "process_metrics")]
    ProcessMetrics(ProcessMetricsResponse),
    #[serde(rename = "journal_entries")]
    JournalEntries(JournalResponse),
}
