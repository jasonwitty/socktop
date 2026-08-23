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
use std::sync::Mutex;
use std::time::Duration as StdDuration;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
#[cfg(feature = "logging")]
use tracing::warn;

// NOTE: CPU normalization env removed; non-Linux now always reports per-process share (0..100) as given by sysinfo.

/// Shared parsing for `/proc/<pid>/stat` (and per-thread `task/<tid>/stat`).
///
/// The second field, `comm`, can contain arbitrary bytes including spaces and
/// parentheses, so naive whitespace splitting mis-parses such names. All
/// callers step past the LAST `')'` and index the remaining space-separated
/// fields from there: 0 = state, 1 = ppid, 11 = utime, 12 = stime,
/// 19 = starttime.
#[cfg(target_os = "linux")]
mod procstat {
    /// Everything after `") "` — the post-comm fields.
    pub fn after_comm(stat: &str) -> Option<&str> {
        stat.get(stat.rfind(')')? + 2..)
    }
    pub fn field(stat: &str, n: usize) -> Option<&str> {
        after_comm(stat)?.split_whitespace().nth(n)
    }
    /// (utime, stime) in clock ticks.
    pub fn utime_stime(stat: &str) -> Option<(u64, u64)> {
        let mut it = after_comm(stat)?.split_whitespace();
        let utime = it.nth(11)?.parse().ok()?;
        let stime = it.next()?.parse().ok()?;
        Some((utime, stime))
    }
    /// One clock tick at USER_HZ=100 (universal on Linux) in microseconds.
    pub const TICK_US: u64 = 10_000;
}

// Read (utime, stime) in MICROSECONDS from /proc/{pid}/stat in one syscall.
// Returns (0, 0) if the file can't be read. Units match the wire contract
// (`DetailedProcessInfo.cpu_time_user` is documented as µs) and the thread
// records — this used to return ms, making process/child CPU times render
// 1000x too small next to thread times.
#[cfg(target_os = "linux")]
fn get_cpu_times_us(pid: u32) -> (u64, u64) {
    let Ok(s) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return (0, 0);
    };
    let Some((utime, stime)) = procstat::utime_stime(&s) else {
        return (0, 0);
    };
    (utime * procstat::TICK_US, stime * procstat::TICK_US)
}

#[cfg(not(target_os = "linux"))]
fn get_cpu_times_us(_pid: u32) -> (u64, u64) {
    (0, 0)
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

// TTL knobs read once at first use, then cached. These hit the hot polling
// paths (every 250ms-1.5s), so re-reading via libc getenv per call is wasted.
fn metrics_ttl_ms() -> u64 {
    static V: OnceCell<u64> = OnceCell::new();
    *V.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_METRICS_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250)
    })
}
fn disks_ttl_ms() -> u64 {
    static V: OnceCell<u64> = OnceCell::new();
    *V.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_DISKS_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_000)
    })
}
#[cfg(target_os = "linux")]
fn processes_ttl_ms() -> u64 {
    static V: OnceCell<u64> = OnceCell::new();
    *V.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_PROCESSES_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_500)
    })
}
#[cfg(not(target_os = "linux"))]
fn name_cache_cleanup_threshold() -> usize {
    static V: OnceCell<usize> = OnceCell::new();
    *V.get_or_init(|| {
        std::env::var("SOCKTOP_AGENT_NAME_CACHE_CLEANUP_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000)
    })
}

// Tiny TTL caches to avoid rescanning sensors every 500ms.
//
// The cached type is Option<...>: a fresh `None` means "we looked recently
// and found nothing" — machines with no matching sensor/GPU no longer rescan
// on every request, only once per TTL.
const TTL: Duration = Duration::from_millis(1500);
static TEMP: crate::state::TtlCell<Option<f32>> = crate::state::TtlCell::new();
static GPUS: crate::state::TtlCell<Option<Vec<crate::gpu::GpuMetrics>>> =
    crate::state::TtlCell::new();

// Gate on `state.components` refreshes (hwmon scans). Both
// collect_fast_metrics and collect_disks need fresh sensor values; without
// this they each paid the hwmon syscall cost on their own cadence. 1s keeps
// disk temps accurate (they change slowly) while suppressing back-to-back
// refreshes from concurrent endpoints.
const COMPONENTS_REFRESH_TTL: Duration = Duration::from_millis(1000);
static COMPONENTS_STAMP: crate::state::TtlCell<()> = crate::state::TtlCell::new();

/// Refresh `state.components` at most once per `COMPONENTS_REFRESH_TTL`.
/// Caller must already hold the components lock.
fn refresh_components_if_stale(components: &mut sysinfo::Components) {
    if COMPONENTS_STAMP.claim_stale(COMPONENTS_REFRESH_TTL) {
        components.refresh(false);
    }
}

// Static caches for unchanging data
static HOSTNAME: OnceCell<String> = OnceCell::new();
struct NetworkNameCache {
    names: Vec<String>,
    infos: Vec<NetworkInfo>,
}
static NETWORK_CACHE: OnceCell<Mutex<NetworkNameCache>> = OnceCell::new();
static CPU_VEC: OnceCell<Mutex<Vec<f32>>> = OnceCell::new();

// Collect only fast-changing metrics (CPU/mem/net + optional temps/gpus).
pub async fn collect_fast_metrics(state: &AppState) -> Metrics {
    let ttl = StdDuration::from_millis(metrics_ttl_ms());
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

    // CPU temperature: only rescan sensors when the cached result (even a
    // cached "no sensor found") goes stale.
    let cpu_temp_c = if !temp_enabled() {
        None
    } else if let Some(cached) = TEMP.get_fresh(TTL) {
        cached
    } else {
        let val = {
            let mut components = state.components.lock().await;
            refresh_components_if_stale(&mut components);
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
        TEMP.set(val);
        val
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

        // Detect a topology change without allocating: compare lengths first,
        // then zip and walk. Only on a real diff do we materialize the new
        // names list. Was: `nets.keys().map(to_string).collect::<Vec<_>>()`
        // every tick — a fresh Vec<String> just to compare.
        let topology_changed = cache.names.len() != nets.keys().count()
            || cache
                .names
                .iter()
                .zip(nets.keys())
                .any(|(cached, current)| cached.as_str() != current.as_str());
        if topology_changed {
            cache.names.clear();
            cache.names.extend(nets.keys().map(|n| n.to_string()));
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

    // GPUs: negative-probe cache short-circuits GPU-less hosts; otherwise the
    // TTL cache answers, and only a stale miss reaches the worker thread.
    let gpus = if !gpu_enabled()
        || (state.gpu_checked.load(std::sync::atomic::Ordering::Acquire)
            && !state.gpu_present.load(std::sync::atomic::Ordering::Relaxed))
    {
        None
    } else if let Some(cached) = GPUS.get_fresh(TTL) {
        cached
    } else {
        let v = collect_all_gpus().await;
        // First probe records presence; subsequent calls rely on the flags.
        if !state
            .gpu_checked
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            state
                .gpu_present
                .store(v.is_some(), std::sync::atomic::Ordering::Release);
        }
        GPUS.set(v.clone());
        v
    };

    let sampled_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let metrics = Metrics {
        sampled_at_ms,
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

/// Best-effort parent-disk name for a partition device name:
/// "nvme0n1p1" -> "nvme0n1", "mmcblk0p2" -> "mmcblk0", "sda1" -> "sda".
/// Works with or without a "/dev/" prefix.
fn parent_disk_name(name: &str) -> &str {
    if let Some(pos) = name.rfind('p') {
        let suffix = &name[pos + 1..];
        if !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()) {
            return &name[..pos];
        }
    }
    name.trim_end_matches(|c: char| c.is_ascii_digit())
}

/// Whether a device name refers to a partition rather than a whole disk.
///
/// On Linux, whole-disk devices are directories under /sys/block and
/// partitions are not, so "is a partition" = "not in /sys/block, but the
/// derived parent is". This gets right the cases the old name heuristic got
/// wrong: a whole-disk filesystem on nvme0n1 (ends in a digit but IS in
/// /sys/block) and zram1 (a whole device). Non-Linux keeps the heuristic.
fn is_partition_name(name: &str) -> bool {
    let bare = name.strip_prefix("/dev/").unwrap_or(name);
    #[cfg(target_os = "linux")]
    {
        let sys_block = std::path::Path::new("/sys/block");
        if sys_block.is_dir() {
            return !sys_block.join(bare).is_dir()
                && sys_block.join(parent_disk_name(bare)).is_dir();
        }
    }
    is_partition_heuristic(bare)
}

/// Name-based fallback for platforms without /sys/block: a p<digits> marker
/// or a trailing non-zero digit.
fn is_partition_heuristic(bare: &str) -> bool {
    bare.contains("p1")
        || bare.contains("p2")
        || bare.contains("p3")
        || bare.ends_with(|c: char| c.is_ascii_digit() && c != '0')
}

// Cached disks
pub async fn collect_disks(state: &AppState) -> Vec<DiskInfo> {
    let ttl = StdDuration::from_millis(disks_ttl_ms());
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
        // Shared TTL-gated refresh: avoids paying the hwmon scan twice when
        // both endpoints converge in the same second.
        refresh_components_if_stale(&mut components);

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

            let is_partition = is_partition_name(&name);

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
            let parent_name = parent_disk_name(&partition.name);

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
            let parent_name = parent_disk_name(&partition.name);

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
    let s = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (utime, stime) = procstat::utime_stime(&s)?;
    Some(utime.saturating_add(stime))
}

/// Collect all processes (Linux): compute CPU% via /proc jiffies delta; sorting moved to client.
#[cfg(target_os = "linux")]
pub async fn collect_processes_all(state: &AppState) -> ProcessesPayload {
    let ttl = StdDuration::from_millis(processes_ttl_ms());
    {
        let cache = state.cache_processes.lock().await;
        if cache.is_fresh(ttl)
            && let Some(c) = cache.get()
        {
            return c.clone();
        }
    }
    // Reuse shared System to avoid reallocation. We only need name + memory
    // from sysinfo here — per-process CPU% is computed below from /proc/{pid}/stat
    // jiffies (see `read_proc_jiffies` + `read_total_jiffies`), so asking sysinfo
    // to gather CPU/exe/cmd/cwd/env per process is wasted /proc traffic on a Pi
    // (was reading /proc/{pid}/{cmdline,exe,cwd,environ,io,status} for every PID
    // on every 2 s poll via `everything()`).
    //
    // `without_tasks()` is REQUIRED: it suppresses per-thread entries in the
    // process map (without it, sysinfo returns one entry per /proc/[tid] —
    // 780+ entries on a typical desktop because of glib/gdbus/Chrome thread
    // pools). The original code paired this with `everything()`; we keep the
    // filter when downgrading to a minimal refresh spec.
    let mut sys_guard = state.sys.lock().await;
    let sys = &mut *sys_guard;
    // `true` = remove processes that no longer exist. With `false`, this
    // long-lived System kept every process it had ever seen: the list grew
    // without bound (21,648 entries on a machine with 289 processes after a
    // few hours of build churn), process_count was meaningless, and — the
    // reason this was found — a process you killed kept its row forever,
    // because the agent went on reporting it. Safe here only because this is
    // `ProcessesToUpdate::All`; with `Some(pids)` it would treat every process
    // outside that list as dead and drop it.
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory().without_tasks(),
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

    // Compute deltas vs last sample. We hold the proc_cpu lock for the whole
    // collection below so we can read+update the per-pid name cache in one
    // critical section.
    let mut tracker = state.proc_cpu.lock().await;
    let last_total = tracker.last_total;
    // Move the old per-pid jiffies map out for delta computation.
    let mut last_map = std::mem::take(&mut tracker.last_per_pid);
    tracker.last_total = total_now;

    // Resolve a name through the per-pid cache. Allocates only on miss.
    let resolve_name =
        |tracker: &mut crate::state::ProcCpuTracker, pid: u32, p: &sysinfo::Process| -> String {
            if let Some(cached) = tracker.names.get(&pid) {
                return cached.clone();
            }
            let new_name = p.name().to_string_lossy().into_owned();
            tracker.names.insert(pid, new_name.clone());
            new_name
        };

    // On first run or if total delta is tiny, report zeros.
    if last_total == 0 || total_now <= last_total {
        let mut procs: Vec<ProcessInfo> = Vec::with_capacity(total_count);
        for p in sys.processes().values() {
            let pid = p.pid().as_u32();
            let name = resolve_name(&mut tracker, pid, p);
            procs.push(ProcessInfo {
                pid,
                name,
                cpu_usage: 0.0,
                mem_bytes: p.memory(),
            });
        }
        // Stash the just-collected jiffies for next call's delta, then prune
        // dead pids from the name cache. Borrowing dance: retain reads
        // `tracker.last_per_pid` through the closure, which conflicts with
        // the mutable borrow of `tracker.names.retain`. Split via split-borrow:
        tracker.last_per_pid = current;
        let crate::state::ProcCpuTracker {
            ref last_per_pid,
            ref mut names,
            ..
        } = *tracker;
        names.retain(|pid, _| last_per_pid.contains_key(pid));
        return ProcessesPayload {
            process_count: total_count,
            top_processes: procs,
        };
    }

    let dt = total_now.saturating_sub(last_total).max(1) as f32;

    let mut procs: Vec<ProcessInfo> = Vec::with_capacity(total_count);
    for p in sys.processes().values() {
        let pid = p.pid().as_u32();
        let now = current.get(&pid).copied().unwrap_or(0);
        let prev = last_map.remove(&pid).unwrap_or(0);
        let du = now.saturating_sub(prev) as f32;
        let cpu = ((du / dt) * 100.0).clamp(0.0, 100.0);
        let name = resolve_name(&mut tracker, pid, p);
        procs.push(ProcessInfo {
            pid,
            name,
            cpu_usage: cpu,
            mem_bytes: p.memory(),
        });
    }
    // Save current jiffies map for next call and prune dead pids from the
    // name cache. `current` is moved here (no clone — that's also #19).
    tracker.last_per_pid = current;
    let crate::state::ProcCpuTracker {
        ref last_per_pid,
        ref mut names,
        ..
    } = *tracker;
    names.retain(|pid, _| last_per_pid.contains_key(pid));
    drop(tracker);

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

        // For active systems, get accurate CPU metrics.
        // `true` = drop processes that have exited; see the Linux path above for
        // what `false` cost us (an ever-growing list that kept reporting dead
        // processes). Correct only because this is `ProcessesToUpdate::All`.
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind.with_cpu());

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
            // e.g., 100% on 2 cores of 8 core system = 25% total CPU.
            // sysinfo reports per-core percentage which EXCEEDS 100 for
            // multi-threaded processes, so clamp AFTER dividing — clamping
            // first truncated e.g. 400%-on-8-cores to 12.5% instead of 50%.
            let raw = p.cpu_usage();
            let total_cpu = (raw / cpu_count.max(1.0)).clamp(0.0, 100.0);

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

        // Clean up old process names cache when it grows too large.
        let cache_cleanup_threshold = name_cache_cleanup_threshold();

        if total_count > proc_cache.names.len() + cache_cleanup_threshold {
            // `now` is only consumed by the `tracing::debug!` below, so gate
            // the binding with the same cfg as its consumer. Without this,
            // a non-logging build (the default) emits an unused-variable
            // warning. The Linux CI doesn't catch it because this block lives
            // in the `#[cfg(not(target_os = "linux"))]` collect_processes_all —
            // the warning only surfaces on the Windows build matrix.
            #[cfg(feature = "logging")]
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

/// Single-read extraction of the /proc/{pid}/status fields the detail
/// endpoint cares about. Callers used to open this file twice per
/// detail-process record (once for VmRSS/VmSize, once for Uid/Gid/Threads/
/// State); now it's one read + one scan.
#[cfg(target_os = "linux")]
#[derive(Default)]
struct ProcStatus {
    rss_kb: u64,
    vsize_kb: u64,
    uid: u32,
    gid: u32,
    threads: u32,
    /// Raw status letter from `State:` (e.g. 'R', 'S'). '?' if missing.
    state_ch: char,
}

#[cfg(target_os = "linux")]
fn read_proc_status(pid: u32) -> Option<ProcStatus> {
    let content = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let mut out = ProcStatus {
        state_ch: '?',
        ..Default::default()
    };
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("VmRSS:") {
            out.rss_kb = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("VmSize:") {
            out.vsize_kb = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Uid:") {
            out.uid = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Gid:") {
            out.gid = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Threads:") {
            out.threads = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("State:") {
            out.state_ch = v.trim().chars().next().unwrap_or('?');
        }
    }
    Some(out)
}

#[cfg(target_os = "linux")]
fn proc_state_label(c: char) -> &'static str {
    match c {
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
    }
}

/// Read parent PID from /proc/{pid}/stat
#[cfg(target_os = "linux")]
fn read_parent_pid_from_proc(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Post-comm field 1 is ppid.
    procstat::field(&stat, 1)?.parse::<u32>().ok()
}

/// Collect process information from /proc files
#[cfg(target_os = "linux")]
fn collect_process_info_from_proc(
    pid: u32,
    system: &sysinfo::System,
) -> Option<DetailedProcessInfo> {
    // One read of /proc/{pid}/status gets us everything the detail endpoint
    // needs from it: memory (when not in sysinfo cache), Uid/Gid, Threads,
    // and State. The previous code opened this file twice per process record.
    let st = read_proc_status(pid)?;

    let (name, cpu_usage, mem_bytes, virtual_mem_bytes) =
        if let Some(proc) = system.process(sysinfo::Pid::from_u32(pid)) {
            (
                proc.name().to_string_lossy().to_string(),
                proc.cpu_usage(),
                proc.memory(),
                proc.virtual_memory(),
            )
        } else {
            // Process not in sysinfo cache — derive name from /proc/{pid}/comm
            // and memory from the status read above.
            let name = fs::read_to_string(format!("/proc/{pid}/comm"))
                .ok()?
                .trim()
                .to_string();
            (name, 0.0, st.rss_kb * 1024, st.vsize_kb * 1024)
        };

    // Read command line
    let command = fs::read_to_string(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|s| s.replace('\0', " ").trim().to_string())
        .unwrap_or_default();

    let uid = st.uid;
    let gid = st.gid;
    let thread_count = st.threads;
    let status = proc_state_label(st.state_ch).to_string();

    // starttime is post-comm field 19.
    let start_time = if let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) {
        procstat::field(&stat, 19)?.parse::<u64>().ok()?
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

    // One read of /proc/{pid}/stat covers both user + system CPU times.
    let (cpu_time_user, cpu_time_system) = get_cpu_times_us(pid);

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
        cpu_time_user,
        cpu_time_system,
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

        // Read thread stat for CPU times and status.
        let stat_path = format!("/proc/{pid}/task/{tid}/stat");
        let Ok(stat_content) = fs::read_to_string(&stat_path) else {
            continue;
        };

        let status = procstat::field(&stat_content, 0)
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

        let (utime, stime) = procstat::utime_stime(&stat_content).unwrap_or((0, 0));
        let cpu_time_user = utime * procstat::TICK_US;
        let cpu_time_system = stime * procstat::TICK_US;

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
        // cmd/exe/cwd feed the modal's Command & Details pane. They're
        // immutable per process, so OnlyIfNotSet reads them once per PID and
        // serves the cache afterwards — the "minimal refresh" optimization
        // had dropped them entirely, leaving the pane blank.
        ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_disk_usage()
            .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_exe(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_cwd(sysinfo::UpdateKind::OnlyIfNotSet),
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

    // Read UID and GID directly from /proc/{pid}/status for accuracy.
    // Uses the shared single-read helper (also extracts memory, threads,
    // state — we discard those here since sysinfo already provided them).
    #[cfg(target_os = "linux")]
    let (user_id, group_id) = read_proc_status(pid)
        .map(|s| (s.uid, s.gid))
        .unwrap_or((0, 0));

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

    // One read of /proc/{pid}/stat covers both user + system CPU times.
    let (cpu_time_user, cpu_time_system) = get_cpu_times_us(pid);

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
        cpu_time_user,
        cpu_time_system,
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

/// Epoch microseconds -> RFC 3339 UTC for display. The old code
/// Debug-formatted a SystemTime and string-replaced it into a raw epoch
/// string that was neither ISO 8601 nor what the field documented.
fn format_journal_timestamp(timestamp_us: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp_nanos(timestamp_us as i128 * 1000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| timestamp_us.to_string())
}

/// Collect journal entries for a specific process.
///
/// Async via tokio::process — journalctl can take hundreds of ms on slow
/// storage, and the old std::process call blocked one of the runtime's two
/// worker threads for the duration.
pub async fn collect_journal_entries(pid: u32) -> Result<JournalResponse, String> {
    let output = tokio::process::Command::new("journalctl")
        .args([
            &format!("_PID={pid}"),
            "--output=json",
            "--lines=100",
            "--no-pager",
        ])
        .output()
        .await
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

        // __REALTIME_TIMESTAMP is epoch microseconds as a string.
        let timestamp_us = json
            .get("__REALTIME_TIMESTAMP")
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        let timestamp = format_journal_timestamp(timestamp_us);

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
            timestamp_us,
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
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp_us));

    // journalctl exits 0 with no output when the invoking user simply cannot
    // SEE the process's entries (e.g. a user-run agent asking about a system
    // service) — but it explains itself on stderr ("You are currently not
    // seeing messages from other users and the system…"). Pass that hint
    // along so the client can distinguish "no logs" from "no access".
    let notice = if entries.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        let hint: String = err
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .take(2)
            .collect::<Vec<_>>()
            .join(" ");
        if hint.is_empty() { None } else { Some(hint) }
    } else {
        None
    };

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
        notice,
        cached_at: response_timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// comm can contain spaces and parens; parsing must key off the LAST ')'.
    #[cfg(target_os = "linux")]
    #[test]
    fn procstat_handles_hostile_comm_names() {
        let stat = "1234 (weird name) (2)) R 1 2 3 4 5 6 7 8 9 10 700 800 0 0 20";
        assert_eq!(procstat::field(stat, 0), Some("R"));
        assert_eq!(procstat::field(stat, 1), Some("1"));
        assert_eq!(procstat::utime_stime(stat), Some((700, 800)));
    }

    /// USER_HZ ticks convert to MICROSECONDS — the wire contract. This used
    /// to be *10 (ms), rendering process CPU times 1000x too small next to
    /// thread times.
    #[cfg(target_os = "linux")]
    #[test]
    fn cpu_times_are_microseconds() {
        assert_eq!(procstat::TICK_US, 10_000);
    }

    #[test]
    fn parent_disk_name_strips_partition_suffixes() {
        assert_eq!(parent_disk_name("nvme0n1p1"), "nvme0n1");
        assert_eq!(parent_disk_name("nvme1n1p12"), "nvme1n1");
        assert_eq!(parent_disk_name("mmcblk0p2"), "mmcblk0");
        assert_eq!(parent_disk_name("sda1"), "sda");
        assert_eq!(parent_disk_name("/dev/nvme0n1p1"), "/dev/nvme0n1");
        // 'p' inside a word is not a partition marker.
        assert_eq!(parent_disk_name("mapper/vg-lv"), "mapper/vg-lv");
    }

    /// The old heuristic flagged whole-disk names ending in a digit
    /// (nvme0n1, zram1) as partitions. On Linux /sys/block decides; this
    /// pins the real-machine behavior for devices every Linux box has.
    #[cfg(target_os = "linux")]
    #[test]
    fn sys_block_devices_are_not_partitions() {
        let sys_block = std::path::Path::new("/sys/block");
        if !sys_block.is_dir() {
            return; // exotic environment; nothing to assert
        }
        for entry in std::fs::read_dir(sys_block).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            assert!(
                !is_partition_name(&name),
                "{name} is a whole disk but was flagged as a partition"
            );
        }
    }

    #[test]
    fn journal_timestamps_are_rfc3339() {
        let s = format_journal_timestamp(1_786_752_000_000_000);
        assert_eq!(s, "2026-08-15T00:00:00Z");
        // Sub-second precision survives.
        let s = format_journal_timestamp(1_786_752_000_123_456);
        assert!(s.starts_with("2026-08-15T00:00:00.123456"), "{s}");
    }
}
