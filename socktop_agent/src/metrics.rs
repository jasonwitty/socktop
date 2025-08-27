//! Metrics collection using sysinfo for socktop_agent.

use crate::gpu::collect_all_gpus;
use crate::state::AppState;
use crate::types::{DiskInfo, Metrics, NetworkInfo, ProcessInfo, ProcessesPayload};
use once_cell::sync::OnceCell;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io;
use std::sync::Mutex;
use std::time::Duration as StdDuration;
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
use tracing::warn;

// NOTE: CPU normalization env removed; non-Linux now always reports per-process share (0..100) as given by sysinfo.
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
    if let Some(lock) = TEMP.get() {
        if let Ok(mut c) = lock.lock() {
            c.v = v;
            c.at = Some(Instant::now());
        }
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
    if let Some(lock) = GPUC.get() {
        if let Ok(mut c) = lock.lock() {
            c.v = v.clone();
            c.at = Some(Instant::now());
        }
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
        if cache.is_fresh(ttl) {
            if let Some(c) = cache.take_clone() {
                return c;
            }
        }
    }
    let mut sys = state.sys.lock().await;
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
    })) {
        warn!("sysinfo selective refresh panicked: {e:?}");
    }

    let hostname = state.hostname.clone();
    let cpu_total = sys.global_cpu_usage();
    let cpu_per_core: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
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

    // Networks
    let networks: Vec<NetworkInfo> = {
        let mut nets = state.networks.lock().await;
        nets.refresh(false);
        nets.iter()
            .map(|(name, data)| NetworkInfo {
                name: name.to_string(),
                received: data.total_received(),
                transmitted: data.total_transmitted(),
            })
            .collect()
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
                Err(e) => {
                    warn!("gpu collection failed: {e}");
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
        if cache.is_fresh(ttl) {
            if let Some(v) = cache.take_clone() {
                return v;
            }
        }
    }
    let mut disks_list = state.disks.lock().await;
    disks_list.refresh(false); // don't drop missing disks
    let disks: Vec<DiskInfo> = disks_list
        .iter()
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();
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
        if cache.is_fresh(ttl) {
            if let Some(v) = cache.take_clone() {
                return v;
            }
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
    // Adaptive TTL based on system load
    let sys_guard = state.sys.lock().await;
    let load = sys_guard.global_cpu_usage();
    drop(sys_guard);

    let ttl_ms: u64 = if let Ok(v) = std::env::var("SOCKTOP_AGENT_PROCESSES_TTL_MS") {
        v.parse().unwrap_or(2_000)
    } else {
        // Adaptive TTL: longer when system is idle
        if load < 10.0 {
            4_000 // Light load
        } else if load < 30.0 {
            2_000 // Medium load
        } else {
            1_000 // High load
        }
    };
    let ttl = StdDuration::from_millis(ttl_ms);

    // Serve from cache if fresh
    {
        let cache = state.cache_processes.lock().await;
        if cache.is_fresh(ttl) {
            if let Some(v) = cache.take_clone() {
                return v;
            }
        }
    }

    // Single efficient refresh: only update processes using significant CPU
    let (total_count, procs) = {
        let mut sys = state.sys.lock().await;
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();

        // Only refresh processes using >0.1% CPU
        sys.refresh_processes_specifics(
            ProcessesToUpdate::new().with_cpu_usage_higher_than(0.1),
            false,
            kind,
        );
        sys.refresh_cpu_usage();

        let total_count = sys.processes().len();

        // Reuse allocations via process cache
        let mut proc_cache = state.proc_cache.lock().await;
        proc_cache.reusable_vec.clear();

        // Filter and collect processes with meaningful CPU usage
        for p in sys.processes().values() {
            let raw = p.cpu_usage();
            if raw > 0.1 {
                // Skip negligible CPU users
                let pid = p.pid().as_u32();

                // Reuse cached name if available
                let name = if let Some(cached) = proc_cache.names.get(&pid) {
                    cached.clone()
                } else {
                    let new_name = p.name().to_string_lossy().into_owned();
                    proc_cache.names.insert(pid, new_name.clone());
                    new_name
                };

                proc_cache.reusable_vec.push(ProcessInfo {
                    pid,
                    name,
                    cpu_usage: raw.clamp(0.0, 100.0),
                    mem_bytes: p.memory(),
                });
            }
        }

        // Clean up old process names periodically
        if total_count > proc_cache.names.len() + 100 {
            proc_cache
                .names
                .retain(|pid, _| sys.processes().contains_key(&sysinfo::Pid::from_u32(*pid)));
        }

        (total_count, proc_cache.reusable_vec.clone())
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
