//! Metrics collection using sysinfo for socktop_agent.

use crate::gpu::collect_all_gpus;
use crate::state::AppState;
use crate::types::{
    DetailedProcessInfo, DiskInfo, JournalEntry, JournalResponse, LogLevel, Metrics, NetworkInfo,
    ProcessInfo, ProcessMetricsResponse, ProcessesPayload,
};
use once_cell::sync::OnceCell;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration as StdDuration;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
#[cfg(feature = "logging")]
use tracing::warn;

// NOTE: CPU normalization env removed; non-Linux now always reports per-process share (0..100) as given by sysinfo.

// Helper functions to get CPU time from /proc/stat on Linux
#[cfg(target_os = "linux")]
fn get_cpu_time_user(pid: u32) -> u64 {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        let fields: Vec<&str> = stat.split_whitespace().collect();
        if fields.len() > 13 {
            // Field 13 (0-indexed) is utime (user CPU time in clock ticks)
            if let Ok(utime) = fields[13].parse::<u64>() {
                // Convert clock ticks to milliseconds (assuming 100 Hz)
                return utime * 10; // 1 tick = 10ms at 100 Hz
            }
        }
    }
    0
}

#[cfg(target_os = "linux")]
fn get_cpu_time_system(pid: u32) -> u64 {
    if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        let fields: Vec<&str> = stat.split_whitespace().collect();
        if fields.len() > 14 {
            // Field 14 (0-indexed) is stime (system CPU time in clock ticks)
            if let Ok(stime) = fields[14].parse::<u64>() {
                // Convert clock ticks to milliseconds (assuming 100 Hz)
                return stime * 10; // 1 tick = 10ms at 100 Hz
            }
        }
    }
    0
}

#[cfg(not(target_os = "linux"))]
fn get_cpu_time_user(_pid: u32) -> u64 {
    0 // Not implemented for non-Linux platforms
}

#[cfg(not(target_os = "linux"))]
fn get_cpu_time_system(_pid: u32) -> u64 {
    0 // Not implemented for non-Linux platforms
}
// Runtime toggles (read once)
fn gpu_enabled() -> bool {
    static ON: OnceCell<bool> = OnceCell::new();
    *ON.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_GPU")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}
fn temp_enabled() -> bool {
    static ON: OnceCell<bool> = OnceCell::new();
    *ON.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_TEMP")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

// Tiny TTL caches to avoid rescanning sensors every 500ms
const TTL: Duration = Duration::from_millis(1500);
struct TempCache {
    at: Option<Instant>,
    v: Option<f32>,
}
static TEMP: OnceCell<Mutex<TempCache>> = OnceCell::new();

struct GpuCache {
    at: Option<Instant>,
    v: Option<Vec<crate::gpu::GpuMetrics>>,
}
static GPUC: OnceCell<Mutex<GpuCache>> = OnceCell::new();

// Static caches for unchanging data
static HOSTNAME: OnceCell<String> = OnceCell::new();
struct NetworkNameCache {
    names: Vec<String>,
    infos: Vec<NetworkInfo>,
}
static NETWORK_CACHE: OnceCell<Mutex<NetworkNameCache>> = OnceCell::new();
static CPU_VEC: OnceCell<Mutex<Vec<f32>>> = OnceCell::new();

fn cached_temp() -> Option<f32> {
    if !temp_enabled() {
        return None;
    }
    let now = Instant::now();
    let lock = TEMP.get_or_init(|| Mutex::new(TempCache { at: None, v: None }));
    let mut c = lock.lock().ok()?;
    if c.at.is_none_or(|t| now.duration_since(t) >= TTL) {
        c.at = Some(now);
        // caller will fill this; we just hold a slot
        c.v = None;
    }
    c.v
}

fn set_temp(v: Option<f32>) {
    if let Some(lock) = TEMP.get()
        && let Ok(mut c) = lock.lock()
    {
        c.v = v;
        c.at = Some(Instant::now());
    }
}

fn cached_gpus() -> Option<Vec<crate::gpu::GpuMetrics>> {
    if !gpu_enabled() {
        return None;
    }
    let now = Instant::now();
    let lock = GPUC.get_or_init(|| Mutex::new(GpuCache { at: None, v: None }));
    let mut c = lock.lock().ok()?;
    if c.at.is_none_or(|t| now.duration_since(t) >= TTL) {
        // mark stale; caller will refresh
        c.at = Some(now);
        c.v = None;
    }
    c.v.clone()
}

fn set_gpus(v: Option<Vec<crate::gpu::GpuMetrics>>) {
    if let Some(lock) = GPUC.get()
        && let Ok(mut c) = lock.lock()
    {
        c.v = v.clone();
        c.at = Some(Instant::now());
    }
}

// Collect only fast-changing metrics (CPU/mem/net + optional temps/gpus).
pub async fn collect_fast_metrics(state: &AppState) -> Metrics {
    // TTL (ms) overridable via env, default 250ms
    let ttl_ms: u64 = std::env::var("SOCKTOP_AGENT_METRICS_TTL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250);
    let ttl = StdDuration::from_millis(ttl_ms);
    {
        let cache = state.cache_metrics.lock().await;
        if cache.is_fresh(ttl)
            && let Some(c) = cache.get()
        {
            return c.clone();
        }
    }
    let mut sys = state.sys.lock().await;
    if let Err(_e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
    })) {
        #[cfg(feature = "logging")]
        warn!("sysinfo selective refresh panicked: {_e:?}");
    }

    // Get or initialize hostname once
    let hostname = HOSTNAME.get_or_init(|| state.hostname.clone()).clone();

    // Reuse CPU vector to avoid allocation
    let cpu_total = sys.global_cpu_usage();
    let cpu_per_core = {
        let vec_lock = CPU_VEC.get_or_init(|| Mutex::new(Vec::with_capacity(32)));
        let mut vec = vec_lock.lock().unwrap();
        vec.clear();
        vec.extend(sys.cpus().iter().map(|c| c.cpu_usage()));
        vec.clone() // Still need to clone but the allocation is reused
    };

    let mem_total = sys.total_memory();
    let mem_used = mem_total.saturating_sub(sys.available_memory());
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();
    drop(sys);

    // CPU temperature: only refresh sensors if cache is stale
    let cpu_temp_c = if cached_temp().is_some() {
        cached_temp()
    } else if temp_enabled() {
        let val = {
            let mut components = state.components.lock().await;
            components.refresh(false);
            components.iter().find_map(|c| {
                let l = c.label().to_ascii_lowercase();
                if l.contains("cpu")
                    || l.contains("package")
                    || l.contains("tctl")
                    || l.contains("tdie")
                {
                    c.temperature()
                } else {
                    None
                }
            })
        };
        set_temp(val);
        val
    } else {
        None
    };

    // Networks with reusable name cache
    let networks = {
        let mut nets = state.networks.lock().await;
        nets.refresh(false);

        // Get or initialize network cache
        let cache = NETWORK_CACHE.get_or_init(|| {
            Mutex::new(NetworkNameCache {
                names: Vec::new(),
                infos: Vec::with_capacity(4), // Most systems have few network interfaces
            })
        });
        let mut cache = cache.lock().unwrap();

        // Collect current network names
        let current_names: Vec<_> = nets.keys().map(|name| name.to_string()).collect();

        // Update cached network names if they changed
        if cache.names != current_names {
            cache.names = current_names;
        }

        // Reuse NetworkInfo objects
        cache.infos.clear();
        for (name, data) in nets.iter() {
            cache.infos.push(NetworkInfo {
                name: name.to_string(), // We'll still clone but avoid Vec reallocation
                received: data.total_received(),
                transmitted: data.total_transmitted(),
            });
        }
        cache.infos.clone()
    };

    // GPUs: if we already determined none exist, short-circuit (no repeated probing)
    let gpus = if gpu_enabled() {
        if state.gpu_checked.load(std::sync::atomic::Ordering::Acquire)
            && !state.gpu_present.load(std::sync::atomic::Ordering::Relaxed)
        {
            None
        } else if cached_gpus().is_some() {
            cached_gpus()
        } else {
            let v = match collect_all_gpus() {
                Ok(v) if !v.is_empty() => Some(v),
                Ok(_) => None,
                Err(_e) => {
                    #[cfg(feature = "logging")]
                    warn!("gpu collection failed: {_e}");
                    None
                }
            };
            // First probe records presence; subsequent calls rely on cache flags.
            if !state
                .gpu_checked
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                if v.is_some() {
                    state
                        .gpu_present
                        .store(true, std::sync::atomic::Ordering::Release);
                } else {
                    state
                        .gpu_present
                        .store(false, std::sync::atomic::Ordering::Release);
                }
            }
            set_gpus(v.clone());
            v
        }
    } else {
        None
    };

    let metrics = Metrics {
        cpu_total,
        cpu_per_core,
        mem_total,
        mem_used,
        swap_total,
        swap_used,
        hostname,
        cpu_temp_c,
        disks: Vec::new(),
        networks,
        top_processes: Vec::new(),
        gpus,
    };
    {
        let mut cache = state.cache_metrics.lock().await;
        cache.set(metrics.clone());
    }
    metrics
}

// Cached disks
pub async fn collect_disks(state: &AppState) -> Vec<DiskInfo> {
    let ttl_ms: u64 = std::env::var("SOCKTOP_AGENT_DISKS_TTL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000);
    let ttl = StdDuration::from_millis(ttl_ms);
    {
        let cache = state.cache_disks.lock().await;
        if cache.is_fresh(ttl)
            && let Some(v) = cache.get()
        {
            return v.clone();
        }
    }
    let mut disks_list = state.disks.lock().await;
    disks_list.refresh(false); // don't drop missing disks

    // Collect disk temperatures from components
    // NVMe temps show up as "Composite" under different chip names
    let disk_temps = {
        let mut components = state.components.lock().await;
        components.refresh(true); // true = refresh values, not just the list

        let mut composite_temps = Vec::new();

        for c in components.iter() {
            let label = c.label().to_ascii_lowercase();

            // Collect all "Composite" temperatures (these are NVMe drives)
            // Labels are like "nvme Composite CT1000N7BSS503" or "nvme Composite Sabrent Rocket 4.0"
            if label.contains("composite")
                && let Some(temp) = c.temperature()
            {
                #[cfg(feature = "logging")]
                tracing::debug!("Found Composite temp: {}°C", temp);
                composite_temps.push(temp);
            }
        }

        // Store composite temps indexed by their order (nvme0n1, nvme1n1, nvme2n1, etc.)
        let mut temps = std::collections::HashMap::new();
        for (idx, temp) in composite_temps.iter().enumerate() {
            let key = format!("nvme{}n1", idx);
            #[cfg(feature = "logging")]
            tracing::debug!("Mapping {} -> {}°C", key, temp);
            temps.insert(key, *temp);
        }
        #[cfg(feature = "logging")]
        tracing::debug!("Final disk_temps map: {:?}", temps);
        temps
    };

    // First collect all partitions from sysinfo, deduplicating by device name
    // (same partition can be mounted at multiple mount points)
    let mut seen_partitions = std::collections::HashSet::new();
    let partitions: Vec<DiskInfo> = disks_list
        .iter()
        .filter_map(|d| {
            let name = d.name().to_string_lossy().into_owned();

            // Skip if we've already seen this partition/device
            if !seen_partitions.insert(name.clone()) {
                return None;
            }

            // Determine if this is a partition
            let is_partition = name.contains("p1")
                || name.contains("p2")
                || name.contains("p3")
                || name.ends_with('1')
                || name.ends_with('2')
                || name.ends_with('3')
                || name.ends_with('4')
                || name.ends_with('5')
                || name.ends_with('6')
                || name.ends_with('7')
                || name.ends_with('8')
                || name.ends_with('9');

            // Try to find temperature for this disk
            let temperature = disk_temps.iter().find_map(|(key, &temp)| {
                if name.starts_with(key) {
                    #[cfg(feature = "logging")]
                    tracing::debug!("Matched {} with key {} -> {}°C", name, key, temp);
                    Some(temp)
                } else {
                    None
                }
            });

            if temperature.is_none() && !name.starts_with("loop") && !name.starts_with("ram") {
                #[cfg(feature = "logging")]
                tracing::debug!("No temperature found for disk: {}", name);
            }

            Some(DiskInfo {
                name,
                total: d.total_space(),
                available: d.available_space(),
                temperature,
                is_partition,
            })
        })
        .collect();

    // Now create parent disk entries by aggregating partition data
    let mut parent_disks: std::collections::HashMap<String, (u64, u64, Option<f32>)> =
        std::collections::HashMap::new();

    for partition in &partitions {
        if partition.is_partition {
            // Extract parent disk name
            // nvme0n1p1 -> nvme0n1, sda1 -> sda, mmcblk0p1 -> mmcblk0
            let parent_name = if let Some(pos) = partition.name.rfind('p') {
                // Check if character after 'p' is a digit
                if partition
                    .name
                    .chars()
                    .nth(pos + 1)
                    .is_some_and(|c| c.is_ascii_digit())
                {
                    &partition.name[..pos]
                } else {
                    // Handle sda1, sdb2, etc (just trim trailing digit)
                    partition.name.trim_end_matches(char::is_numeric)
                }
            } else {
                // Handle sda1, sdb2, etc (just trim trailing digit)
                partition.name.trim_end_matches(char::is_numeric)
            };

            // Look up temperature for the PARENT disk, not the partition
            // Strip /dev/ prefix if present for matching
            let parent_name_for_match = parent_name.strip_prefix("/dev/").unwrap_or(parent_name);
            let parent_temp = disk_temps.iter().find_map(|(key, &temp)| {
                if parent_name_for_match.starts_with(key) {
                    Some(temp)
                } else {
                    None
                }
            });

            // Aggregate partition stats into parent
            let entry = parent_disks
                .entry(parent_name.to_string())
                .or_insert((0, 0, parent_temp));
            entry.0 += partition.total;
            entry.1 += partition.available;
            // Keep temperature if any partition has it (or if we just found one)
            if entry.2.is_none() {
                entry.2 = parent_temp;
            }
        }
    }

    // Create parent disk entries
    let mut disks: Vec<DiskInfo> = parent_disks
        .into_iter()
        .map(|(name, (total, available, temperature))| DiskInfo {
            name,
            total,
            available,
            temperature,
            is_partition: false,
        })
        .collect();

    // Sort parent disks by name
    disks.sort_by(|a, b| a.name.cmp(&b.name));

    // Add partitions after their parent disk
    for partition in partitions {
        if partition.is_partition {
            // Find parent disk index
            let parent_name = if let Some(pos) = partition.name.rfind('p') {
                if partition
                    .name
                    .chars()
                    .nth(pos + 1)
                    .is_some_and(|c| c.is_ascii_digit())
                {
                    &partition.name[..pos]
                } else {
                    partition.name.trim_end_matches(char::is_numeric)
                }
            } else {
                partition.name.trim_end_matches(char::is_numeric)
            };

            // Find where to insert this partition (after its parent)
            if let Some(parent_idx) = disks.iter().position(|d| d.name == parent_name) {
                // Insert after parent and any existing partitions of that parent
                let mut insert_idx = parent_idx + 1;
                while insert_idx < disks.len()
                    && disks[insert_idx].is_partition
                    && disks[insert_idx].name.starts_with(parent_name)
                {
                    insert_idx += 1;
                }
                disks.insert(insert_idx, partition);
            } else {
                // Parent not found (shouldn't happen), just add at end
                disks.push(partition);
            }
        } else {
            // Not a partition (e.g., zram0), add at end
            disks.push(partition);
        }
    }
    {
        let mut cache = state.cache_disks.lock().await;
        cache.set(disks.clone());
    }
    disks
}

// Linux-only helpers and implementation using /proc deltas for accurate CPU%.
#[cfg(target_os = "linux")]
#[inline]
fn read_total_jiffies() -> io::Result<u64> {
    // /proc/stat first line: "cpu  user nice system idle iowait irq softirq steal ..."
    let s = fs::read_to_string("/proc/stat")?;
    if let Some(line) = s.lines().next() {
        let mut it = line.split_whitespace();
        let _cpu = it.next(); // "cpu"
        let mut sum: u64 = 0;
        for tok in it.take(8) {
            if let Ok(v) = tok.parse::<u64>() {
                sum = sum.saturating_add(v);
            }
        }
        return Ok(sum);
    }
    Err(io::Error::other("no cpu line"))
}

#[cfg(target_os = "linux")]
#[inline]
fn read_proc_jiffies(pid: u32) -> Option<u64> {
    let path = format!("/proc/{pid}/stat");
    let s = fs::read_to_string(path).ok()?;
    // Find the right parenthesis that terminates comm; everything after is space-separated fields starting at "state"
    let rpar = s.rfind(')')?;
    let after = s.get(rpar + 2..)?; // skip ") "
    let mut it = after.split_whitespace();
    // utime (14th field) is offset 11 from "state", stime (15th) is next
    let utime = it.nth(11)?.parse::<u64>().ok()?;
    let stime = it.next()?.parse::<u64>().ok()?;
    Some(utime.saturating_add(stime))
}

/// Collect all processes (Linux): compute CPU% via /proc jiffies delta; sorting moved to client.
#[cfg(target_os = "linux")]
pub async fn collect_processes_all(state: &AppState) -> ProcessesPayload {
    let ttl_ms: u64 = std::env::var("SOCKTOP_AGENT_PROCESSES_TTL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        // Higher default (1500ms) on non-Linux only; keep 1500 here for Linux correctness (more frequent updates).
        .unwrap_or(1_500);
    let ttl = StdDuration::from_millis(ttl_ms);
    {
        let cache = state.cache_processes.lock().await;
        if cache.is_fresh(ttl)
            && let Some(c) = cache.get()
        {
            return c.clone();
        }
    }
    // Reuse shared System to avoid reallocation; refresh processes fully.
    let mut sys_guard = state.sys.lock().await;
    let sys = &mut *sys_guard;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        false,
        ProcessRefreshKind::everything().without_tasks(),
    );

    let total_count = sys.processes().len();

    // Snapshot current per-pid jiffies
    let mut current: HashMap<u32, u64> = HashMap::with_capacity(total_count);
    for p in sys.processes().values() {
        let pid = p.pid().as_u32();
        if let Some(j) = read_proc_jiffies(pid) {
            current.insert(pid, j);
        }
    }
    let total_now = read_total_jiffies().unwrap_or(0);

    // Compute deltas vs last sample
    let (last_total, mut last_map) = {
        #[cfg(target_os = "linux")]
        {
            let mut t = state.proc_cpu.lock().await;
            let lt = t.last_total;
            let lm = std::mem::take(&mut t.last_per_pid);
            t.last_total = total_now;
            t.last_per_pid = current.clone();
            (lt, lm)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _: u64 = total_now; // silence unused warning
            (0u64, HashMap::new())
        }
    };

    // On first run or if total delta is tiny, report zeros
    if last_total == 0 || total_now <= last_total {
        let procs: Vec<ProcessInfo> = sys
            .processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: 0.0,
                mem_bytes: p.memory(),
            })
            .collect();
        return ProcessesPayload {
            process_count: total_count,
            top_processes: procs,
        };
    }

    let dt = total_now.saturating_sub(last_total).max(1) as f32;

    let procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| {
            let pid = p.pid().as_u32();
            let now = current.get(&pid).copied().unwrap_or(0);
            let prev = last_map.remove(&pid).unwrap_or(0);
            let du = now.saturating_sub(prev) as f32;
            let cpu = ((du / dt) * 100.0).clamp(0.0, 100.0);
            ProcessInfo {
                pid,
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: cpu,
                mem_bytes: p.memory(),
            }
        })
        .collect();

    let payload = ProcessesPayload {
        process_count: total_count,
        top_processes: procs,
    };
    {
        let mut cache = state.cache_processes.lock().await;
        cache.set(payload.clone());
    }
    payload
}

/// Collect all processes (non-Linux): optimized for reduced allocations and selective updates.
#[cfg(not(target_os = "linux"))]
pub async fn collect_processes_all(state: &AppState) -> ProcessesPayload {
    // Serve from cache if fresh
    {
        let cache = state.cache_processes.lock().await;
        if cache.is_fresh(StdDuration::from_millis(2_000)) {
            // Use fixed TTL for cache check
            if let Some(c) = cache.get() {
                return c.clone();
            }
        }
    }

    // Single efficient refresh with optimized CPU collection
    let (total_count, procs) = {
        let mut sys = state.sys.lock().await;
        let kind = ProcessRefreshKind::nothing().with_memory();

        // Optimize refresh strategy based on system load
        //if load > 5.0 {

        //JW too complicated. simplify to remove strange behavior

        // For active systems, get accurate CPU metrics
        sys.refresh_processes_specifics(ProcessesToUpdate::All, false, kind.with_cpu());

        // } else {
        //     // For idle systems, just get basic process info
        //     sys.refresh_processes_specifics(ProcessesToUpdate::All, false, kind);
        //     sys.refresh_cpu_usage();
        // }

        let total_count = sys.processes().len();
        let cpu_count = sys.cpus().len() as f32;

        // Reuse allocations via process cache
        let mut proc_cache = state.proc_cache.lock().await;
        proc_cache.reusable_vec.clear();

        // Collect all processes, will sort by CPU later
        for p in sys.processes().values() {
            let pid = p.pid().as_u32();

            // Reuse cached name if available
            let name = if let Some(cached) = proc_cache.names.get(&pid) {
                cached.clone()
            } else {
                let new_name = p.name().to_string_lossy().into_owned();
                proc_cache.names.insert(pid, new_name.clone());
                new_name
            };

            // Convert to percentage of total CPU capacity
            // e.g., 100% on 2 cores of 8 core system = 25% total CPU
            let raw = p.cpu_usage(); // This is per-core percentage
            let total_cpu = raw.clamp(0.0, 100.0) / cpu_count;

            proc_cache.reusable_vec.push(ProcessInfo {
                pid,
                name,
                cpu_usage: total_cpu,
                mem_bytes: p.memory(),
            });
        }

        //JW no need to sort here; client does the sorting

        // // Sort by CPU usage
        // proc_cache.reusable_vec.sort_by(|a, b| {
        //     b.cpu_usage
        //         .partial_cmp(&a.cpu_usage)
        //         .unwrap_or(std::cmp::Ordering::Equal)
        // });

        // Clean up old process names cache when it grows too large
        let cache_cleanup_threshold = std::env::var("SOCKTOP_AGENT_NAME_CACHE_CLEANUP_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000); // Default: most modern systems have 400-700 processes

        if total_count > proc_cache.names.len() + cache_cleanup_threshold {
            let now = std::time::Instant::now();
            proc_cache
                .names
                .retain(|pid, _| sys.processes().contains_key(&sysinfo::Pid::from_u32(*pid)));
            #[cfg(feature = "logging")]
            tracing::debug!(
                "Cleaned up {} stale process names in {}ms",
                proc_cache.names.capacity() - proc_cache.names.len(),
                now.elapsed().as_millis()
            );
        }

        // Get all processes, take ownership of the vec (will be replaced with empty vec)
        (total_count, std::mem::take(&mut proc_cache.reusable_vec))
    };

    let payload = ProcessesPayload {
        process_count: total_count,
        top_processes: procs,
    };

    {
        let mut cache = state.cache_processes.lock().await;
        cache.set(payload.clone());
    }
    payload
}

/// Lightweight child process enumeration using direct /proc access
/// This avoids the expensive refresh_processes_specifics(All) call
#[cfg(target_os = "linux")]
fn enumerate_child_processes_lightweight(
    parent_pid: u32,
    system: &sysinfo::System,
) -> Vec<DetailedProcessInfo> {
    let mut children = Vec::new();

    // Read /proc to find all child processes
    // This is much faster than refresh_processes_specifics(All)
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string()
                && let Ok(pid) = file_name.parse::<u32>()
                && let Some(child_parent_pid) = read_parent_pid_from_proc(pid)
                && child_parent_pid == parent_pid
                && let Some(child_info) = collect_process_info_from_proc(pid, system)
            {
                children.push(child_info);
            }
        }
    }

    children
}

/// Read parent PID from /proc/{pid}/stat
#[cfg(target_os = "linux")]
fn read_parent_pid_from_proc(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Format: pid (comm) state ppid ...
    // We need to handle process names with spaces/parentheses
    let ppid_start = stat.rfind(')')?;
    let fields: Vec<&str> = stat[ppid_start + 1..].split_whitespace().collect();
    // After the closing paren: state ppid ...
    // Field 1 (0-indexed) is ppid
    fields.get(1)?.parse::<u32>().ok()
}

/// Collect process information from /proc files
#[cfg(target_os = "linux")]
fn collect_process_info_from_proc(
    pid: u32,
    system: &sysinfo::System,
) -> Option<DetailedProcessInfo> {
    // Try to get basic info from sysinfo if it's already loaded (cheap lookup)
    // Otherwise read from /proc directly
    let (name, cpu_usage, mem_bytes, virtual_mem_bytes) =
        if let Some(proc) = system.process(sysinfo::Pid::from_u32(pid)) {
            (
                proc.name().to_string_lossy().to_string(),
                proc.cpu_usage(),
                proc.memory(),
                proc.virtual_memory(),
            )
        } else {
            // Process not in sysinfo cache, read minimal info from /proc
            let name = fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()?
                .trim()
                .to_string();

            // Read memory from /proc/{pid}/status
            let status_content = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
            let mut mem_bytes = 0u64;
            let mut virtual_mem_bytes = 0u64;

            for line in status_content.lines() {
                if let Some(value) = line.strip_prefix("VmRSS:") {
                    if let Some(kb) = value.split_whitespace().next() {
                        mem_bytes = kb.parse::<u64>().unwrap_or(0) * 1024;
                    }
                } else if let Some(value) = line.strip_prefix("VmSize:")
                    && let Some(kb) = value.split_whitespace().next()
                {
                    virtual_mem_bytes = kb.parse::<u64>().unwrap_or(0) * 1024;
                }
            }

            (name, 0.0, mem_bytes, virtual_mem_bytes)
        };

    // Read command line
    let command = fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|s| s.replace('\0', " ").trim().to_string())
        .unwrap_or_default();

    // Read status information
    let status_content = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let mut uid = 0u32;
    let mut gid = 0u32;
    let mut thread_count = 0u32;
    let mut status = "Unknown".to_string();

    for line in status_content.lines() {
        if let Some(value) = line.strip_prefix("Uid:") {
            if let Some(uid_str) = value.split_whitespace().next() {
                uid = uid_str.parse().unwrap_or(0);
            }
        } else if let Some(value) = line.strip_prefix("Gid:") {
            if let Some(gid_str) = value.split_whitespace().next() {
                gid = gid_str.parse().unwrap_or(0);
            }
        } else if let Some(value) = line.strip_prefix("Threads:") {
            thread_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("State:") {
            status = value
                .trim()
                .chars()
                .next()
                .map(|c| match c {
                    'R' => "Running",
                    'S' => "Sleeping",
                    'D' => "Disk Sleep",
                    'Z' => "Zombie",
                    'T' => "Stopped",
                    't' => "Tracing Stop",
                    'X' | 'x' => "Dead",
                    'K' => "Wakekill",
                    'W' => "Waking",
                    'P' => "Parked",
                    'I' => "Idle",
                    _ => "Unknown",
                })
                .unwrap_or("Unknown")
                .to_string();
        }
    }

    // Read start time from stat
    let start_time = if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        let stat_end = stat.rfind(')')?;
        let fields: Vec<&str> = stat[stat_end + 1..].split_whitespace().collect();
        // Field 19 (0-indexed) is starttime in clock ticks since boot
        fields.get(19)?.parse::<u64>().ok()?
    } else {
        0
    };

    // Read I/O stats if available
    let (read_bytes, write_bytes) =
        if let Ok(io_content) = fs::read_to_string(format!("/proc/{pid}/io")) {
            let mut read_bytes = None;
            let mut write_bytes = None;

            for line in io_content.lines() {
                if let Some(value) = line.strip_prefix("read_bytes:") {
                    read_bytes = value.trim().parse().ok();
                } else if let Some(value) = line.strip_prefix("write_bytes:") {
                    write_bytes = value.trim().parse().ok();
                }
            }

            (read_bytes, write_bytes)
        } else {
            (None, None)
        };

    // Read working directory
    let working_directory = fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    // Read executable path
    let executable_path = fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    Some(DetailedProcessInfo {
        pid,
        name,
        command,
        cpu_usage,
        mem_bytes,
        virtual_mem_bytes,
        shared_mem_bytes: None, // Would need to parse /proc/{pid}/statm for this
        thread_count,
        fd_count: None, // Would need to count entries in /proc/{pid}/fd
        status,
        parent_pid: None, // We already know the parent
        user_id: uid,
        group_id: gid,
        start_time,
        cpu_time_user: get_cpu_time_user(pid),
        cpu_time_system: get_cpu_time_system(pid),
        read_bytes,
        write_bytes,
        working_directory,
        executable_path,
        child_processes: Vec::new(), // Don't recurse
        threads: Vec::new(),         // Not collected for child processes
    })
}

/// Fallback for non-Linux: use sysinfo (less efficient but functional)
#[cfg(not(target_os = "linux"))]
fn enumerate_child_processes_lightweight(
    parent_pid: u32,
    system: &sysinfo::System,
) -> Vec<DetailedProcessInfo> {
    let mut children = Vec::new();

    // On non-Linux, we have to iterate through all processes in sysinfo
    // This is less efficient but maintains cross-platform compatibility
    for (child_pid, child_process) in system.processes() {
        if let Some(parent) = child_process.parent()
            && parent.as_u32() == parent_pid
        {
            let child_info = DetailedProcessInfo {
                pid: child_pid.as_u32(),
                name: child_process.name().to_string_lossy().to_string(),
                command: child_process
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                cpu_usage: child_process.cpu_usage(),
                mem_bytes: child_process.memory(),
                virtual_mem_bytes: child_process.virtual_memory(),
                shared_mem_bytes: None,
                thread_count: child_process
                    .tasks()
                    .map(|tasks| tasks.len() as u32)
                    .unwrap_or(0),
                fd_count: None,
                status: format!("{:?}", child_process.status()),
                parent_pid: Some(parent_pid),
                // On non-Linux platforms, sysinfo UID/GID might not be accurate
                // Just use 0 as placeholder since we can't read /proc
                user_id: 0,
                group_id: 0,
                start_time: child_process.start_time(),
                cpu_time_user: 0, // Not available on non-Linux in our implementation
                cpu_time_system: 0,
                read_bytes: Some(child_process.disk_usage().read_bytes),
                write_bytes: Some(child_process.disk_usage().written_bytes),
                working_directory: child_process.cwd().map(|p| p.to_string_lossy().to_string()),
                executable_path: child_process.exe().map(|p| p.to_string_lossy().to_string()),
                child_processes: Vec::new(),
                threads: Vec::new(), // Not collected for non-Linux
            };
            children.push(child_info);
        }
    }

    children
}

/// Collect thread information for a specific process (Linux only)
#[cfg(target_os = "linux")]
fn collect_thread_info(pid: u32) -> Vec<crate::types::ThreadInfo> {
    let mut threads = Vec::new();

    // Read /proc/{pid}/task directory
    let task_dir = format!("/proc/{pid}/task");
    let Ok(entries) = fs::read_dir(&task_dir) else {
        return threads;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let tid_str = file_name.to_string_lossy();
        let Ok(tid) = tid_str.parse::<u32>() else {
            continue;
        };

        // Read thread name from comm
        let name = fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm"))
            .unwrap_or_else(|_| format!("Thread-{tid}"))
            .trim()
            .to_string();

        // Read thread stat for CPU times and status
        let stat_path = format!("/proc/{pid}/task/{tid}/stat");
        let Ok(stat_content) = fs::read_to_string(&stat_path) else {
            continue;
        };

        // Parse stat file (similar format to process stat)
        // Fields: pid comm state ... utime stime ...
        let fields: Vec<&str> = stat_content.split_whitespace().collect();
        if fields.len() < 15 {
            continue;
        }

        // Field 2 is state (R, S, D, Z, T, etc.)
        let status = fields
            .get(2)
            .and_then(|s| s.chars().next())
            .map(|c| match c {
                'R' => "Running",
                'S' => "Sleeping",
                'D' => "Disk Sleep",
                'Z' => "Zombie",
                'T' => "Stopped",
                't' => "Tracing Stop",
                'X' | 'x' => "Dead",
                _ => "Unknown",
            })
            .unwrap_or("Unknown")
            .to_string();

        // Field 13 is utime (user CPU time in clock ticks)
        // Field 14 is stime (system CPU time in clock ticks)
        let utime = fields
            .get(13)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let stime = fields
            .get(14)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        // Convert clock ticks to microseconds (assuming 100 Hz)
        // 1 tick = 10ms = 10,000 microseconds
        let cpu_time_user = utime * 10_000;
        let cpu_time_system = stime * 10_000;

        threads.push(crate::types::ThreadInfo {
            tid,
            name,
            cpu_time_user,
            cpu_time_system,
            status,
        });
    }

    threads
}

/// Fallback for non-Linux: return empty thread list
#[cfg(not(target_os = "linux"))]
fn collect_thread_info(_pid: u32) -> Vec<crate::types::ThreadInfo> {
    Vec::new()
}

/// Collect detailed metrics for a specific process
pub async fn collect_process_metrics(
    pid: u32,
    state: &AppState,
) -> Result<ProcessMetricsResponse, String> {
    let mut system = state.sys.lock().await;

    // OPTIMIZED: Only refresh the specific process we care about
    // This avoids polluting the main process list with threads and prevents race conditions
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        false,
        ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_disk_usage(),
    );

    let process = system
        .process(sysinfo::Pid::from_u32(pid))
        .ok_or_else(|| format!("Process {pid} not found"))?;

    // Get current timestamp
    let cached_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Time error: {e}"))?
        .as_secs();

    // Extract all needed data from process while we have the lock
    let name = process.name().to_string_lossy().to_string();
    let command = process
        .cmd()
        .iter()
        .map(|s| s.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let cpu_usage = process.cpu_usage();
    let mem_bytes = process.memory();
    let virtual_mem_bytes = process.virtual_memory();
    let thread_count = process.tasks().map(|tasks| tasks.len() as u32).unwrap_or(0);
    let status = format!("{:?}", process.status());
    let parent_pid = process.parent().map(|p| p.as_u32());
    let start_time = process.start_time();

    // Read UID and GID directly from /proc/{pid}/status for accuracy
    #[cfg(target_os = "linux")]
    let (user_id, group_id) =
        if let Ok(status_content) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
            let mut uid = 0u32;
            let mut gid = 0u32;

            for line in status_content.lines() {
                if let Some(value) = line.strip_prefix("Uid:") {
                    // Uid line format: "Uid:	1000	1000	1000	1000" (real, effective, saved, filesystem)
                    // We want the real UID (first value)
                    if let Some(uid_str) = value.split_whitespace().next() {
                        uid = uid_str.parse().unwrap_or(0);
                    }
                } else if let Some(value) = line.strip_prefix("Gid:") {
                    // Gid line format: "Gid:	1000	1000	1000	1000" (real, effective, saved, filesystem)
                    // We want the real GID (first value)
                    if let Some(gid_str) = value.split_whitespace().next() {
                        gid = gid_str.parse().unwrap_or(0);
                    }
                }
            }

            (uid, gid)
        } else {
            // Fallback if /proc read fails (permission issue)
            (0, 0)
        };

    #[cfg(not(target_os = "linux"))]
    let (user_id, group_id) = (0, 0);

    // Read I/O stats directly from /proc/{pid}/io
    // Use rchar/wchar to capture ALL I/O including cached reads (like htop/btop do)
    // sysinfo's total_read_bytes/total_written_bytes only count actual disk I/O
    #[cfg(target_os = "linux")]
    let (read_bytes, write_bytes) =
        if let Ok(io_content) = std::fs::read_to_string(format!("/proc/{pid}/io")) {
            let mut rchar = 0u64;
            let mut wchar = 0u64;

            for line in io_content.lines() {
                if let Some(value) = line.strip_prefix("rchar: ") {
                    rchar = value.trim().parse().unwrap_or(0);
                } else if let Some(value) = line.strip_prefix("wchar: ") {
                    wchar = value.trim().parse().unwrap_or(0);
                }
            }

            (Some(rchar), Some(wchar))
        } else {
            // Fallback to sysinfo if we can't read /proc (permissions)
            let disk_usage = process.disk_usage();
            (
                Some(disk_usage.total_read_bytes),
                Some(disk_usage.total_written_bytes),
            )
        };

    #[cfg(not(target_os = "linux"))]
    let (read_bytes, write_bytes) = {
        let disk_usage = process.disk_usage();
        (
            Some(disk_usage.total_read_bytes),
            Some(disk_usage.total_written_bytes),
        )
    };

    let working_directory = process.cwd().map(|p| p.to_string_lossy().to_string());
    let executable_path = process.exe().map(|p| p.to_string_lossy().to_string());

    // Collect child processes using lightweight /proc access
    // This avoids the expensive system.refresh_processes_specifics(All) call
    let child_processes = enumerate_child_processes_lightweight(pid, &system);

    // Release the system lock early (automatic when system goes out of scope)
    drop(system);

    // Collect thread information (Linux only)
    let threads = collect_thread_info(pid);

    // Now construct the detailed info without holding the lock
    let detailed_info = DetailedProcessInfo {
        pid,
        name,
        command,
        cpu_usage,
        mem_bytes,
        virtual_mem_bytes,
        shared_mem_bytes: None, // Not available from sysinfo
        thread_count,
        fd_count: None, // Not available from sysinfo on all platforms
        status,
        parent_pid,
        user_id,
        group_id,
        start_time,
        cpu_time_user: get_cpu_time_user(pid),
        cpu_time_system: get_cpu_time_system(pid),
        read_bytes,
        write_bytes,
        working_directory,
        executable_path,
        child_processes,
        threads,
    };

    Ok(ProcessMetricsResponse {
        process: detailed_info,
        cached_at,
    })
}

/// Collect journal entries for a specific process
pub fn collect_journal_entries(pid: u32) -> Result<JournalResponse, String> {
    let output = Command::new("journalctl")
        .args([
            &format!("_PID={pid}"),
            "--output=json",
            "--lines=100",
            "--no-pager",
        ])
        .output()
        .map_err(|e| format!("Failed to execute journalctl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "journalctl failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    // Parse each line as JSON (journalctl outputs one JSON object per line)
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let json: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("Failed to parse journal JSON: {e}"))?;

        // Extract relevant fields
        let timestamp_str = json
            .get("__REALTIME_TIMESTAMP")
            .and_then(|v| v.as_str())
            .unwrap_or("0");

        // Convert timestamp to ISO 8601 format
        let timestamp = if let Ok(ts_micros) = timestamp_str.parse::<u64>() {
            let ts_secs = ts_micros / 1_000_000;
            let ts_nanos = (ts_micros % 1_000_000) * 1000;
            let time = SystemTime::UNIX_EPOCH
                + Duration::from_secs(ts_secs)
                + Duration::from_nanos(ts_nanos);
            // Simple ISO 8601 format - we can improve this if needed
            format!("{time:?}")
                .replace("SystemTime { tv_sec: ", "")
                .replace(", tv_nsec: ", ".")
                .replace(" }", "")
        } else {
            timestamp_str.to_string()
        };

        let priority = match json.get("PRIORITY").and_then(|v| v.as_str()) {
            Some("0") => LogLevel::Emergency,
            Some("1") => LogLevel::Alert,
            Some("2") => LogLevel::Critical,
            Some("3") => LogLevel::Error,
            Some("4") => LogLevel::Warning,
            Some("5") => LogLevel::Notice,
            Some("6") => LogLevel::Info,
            Some("7") => LogLevel::Debug,
            _ => LogLevel::Info,
        };

        let message = json
            .get("MESSAGE")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let unit = json
            .get("_SYSTEMD_UNIT")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let entry_pid = json
            .get("_PID")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok());

        let comm = json
            .get("_COMM")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let uid = json
            .get("_UID")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok());

        let gid = json
            .get("_GID")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok());

        entries.push(JournalEntry {
            timestamp,
            priority,
            message,
            unit,
            pid: entry_pid,
            comm,
            uid,
            gid,
        });
    }

    // Sort by timestamp (newest first)
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let response_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Time error: {e}"))?
        .as_secs();

    let total_count = entries.len() as u32;
    let truncated = entries.len() >= 100; // We requested 100 lines, so if we got 100, there might be more

    Ok(JournalResponse {
        entries,
        total_count,
        truncated,
        cached_at: response_timestamp,
    })
}
