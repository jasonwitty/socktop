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
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tracing::warn;

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
    let mut sys = state.sys.lock().await;
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
    })) {
        warn!("sysinfo selective refresh panicked: {e:?}");
    }

    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
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

    // GPUs: refresh only when cache is stale
    let gpus = if cached_gpus().is_some() {
        cached_gpus()
    } else if gpu_enabled() {
        let v = match collect_all_gpus() {
            Ok(v) if !v.is_empty() => Some(v),
            Ok(_) => None,
            Err(e) => {
                warn!("gpu collection failed: {e}");
                None
            }
        };
        set_gpus(v.clone());
        v
    } else {
        None
    };

    Metrics {
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
    }
}

// Cached disks
pub async fn collect_disks(state: &AppState) -> Vec<DiskInfo> {
    let mut disks_list = state.disks.lock().await;
    disks_list.refresh(false); // don't drop missing disks
    disks_list
        .iter()
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect()
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

/// Collect top processes (Linux variant): compute CPU% via /proc jiffies delta.
#[cfg(target_os = "linux")]
pub async fn collect_processes_top_k(state: &AppState, k: usize) -> ProcessesPayload {
    // Fresh view to avoid lingering entries and select "no tasks" (no per-thread rows).
    let mut sys = System::new();
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
            top_processes: top_k_sorted(procs, k),
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

    ProcessesPayload {
        process_count: total_count,
        top_processes: top_k_sorted(procs, k),
    }
}

/// Collect top processes (non-Linux): use sysinfo's internal CPU% by doing a double refresh.
#[cfg(not(target_os = "linux"))]
pub async fn collect_processes_top_k(state: &AppState, k: usize) -> ProcessesPayload {
    use tokio::time::sleep;

    let mut sys = state.sys.lock().await;

    // First refresh to set baseline
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        false,
        ProcessRefreshKind::everything().without_tasks(),
    );
    // Small delay so sysinfo can compute CPU deltas on next refresh
    sleep(Duration::from_millis(250)).await;
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        false,
        ProcessRefreshKind::everything().without_tasks(),
    );

    let total_count = sys.processes().len();

    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            cpu_usage: p.cpu_usage(),
            mem_bytes: p.memory(),
        })
        .collect();

    procs = top_k_sorted(procs, k);
    ProcessesPayload {
        process_count: total_count,
        top_processes: procs,
    }
}

// Small helper to select and sort top-k by cpu
fn top_k_sorted(mut v: Vec<ProcessInfo>, k: usize) -> Vec<ProcessInfo> {
    if v.len() > k {
        v.select_nth_unstable_by(k, |a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v.truncate(k);
    }
    v.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    v
}
