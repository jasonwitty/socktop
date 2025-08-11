//! Metrics collection using sysinfo. Keeps sysinfo handles in AppState to
//! avoid repeated allocations and allow efficient refreshes.


use crate::gpu::collect_all_gpus;



use crate::state::AppState;
use crate::types::{DiskInfo, Metrics, NetworkInfo, ProcessInfo};


use sysinfo::{
    System, Components,
    ProcessRefreshKind, RefreshKind, MemoryRefreshKind, CpuRefreshKind, DiskRefreshKind,
    NetworkRefreshKind,
};
use tracing::{warn, error};

pub async fn collect_metrics(state: &AppState) -> Metrics {
    // Lock sysinfo once; if poisoned, recover inner.
    let mut sys = match state.sys.lock().await {
        guard => guard, // Mutex from tokio::sync doesn't poison; this is safe
    };

    // Refresh pieces (avoid heavy refresh_all if you already call periodically).
    // Wrap in catch_unwind in case a crate panics internally.
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Newer sysinfo (0.36.x) wants explicit refresh kinds.
        // Build a minimal RefreshKind instead of refresh_all() to keep it light.
        let rk = RefreshKind::new()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::new())
            .with_disks(DiskRefreshKind::everything())
            .with_networks(NetworkRefreshKind::everything())
            .with_components(); // temps

        sys.refresh_specifics(rk);

        // Processes: need a separate call with the desired per‑process fields.
        let prk = ProcessRefreshKind::new()
            .with_cpu()
            .with_memory()
            .with_disk_usage(); // add/remove as needed
        sys.refresh_processes_specifics(prk, |_| true, true);
    })) {
        warn!("system refresh panicked: {:?}", e);
    }

    // Hostname
    let hostname = sys.host_name().unwrap_or_else(|| "unknown".to_string());

    // CPU total & per-core
    let cpu_total = sys.global_cpu_info().cpu_usage();
    let cpu_per_core: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();

    // Memory / swap
    let mem_total = sys.total_memory();
    let mem_used  = mem_total.saturating_sub(sys.available_memory());
    let swap_total = sys.total_swap();
    let swap_used  = sys.used_swap();

    // Temperature (first CPU-like component if any)
    let cpu_temp_c = sys
        .components()
        .iter()
        .filter(|c| {
            let l = c.label().to_ascii_lowercase();
            l.contains("cpu") || l.contains("package") || l.contains("core 0")
        })
        .map(|c| c.temperature() as f32)
        .next();

    // Disks
    let disks: Vec<Disk> = sys
        .disks()
        .iter()
        .map(|d| Disk {
            name: d.name().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();

    // Networks (cumulative)
    let networks: Vec<Network> = sys
        .networks()
        .iter()
        .map(|(_, data)| Network {
            received: data.received(),
            transmitted: data.transmitted(),
        })
        .collect();

    // Processes (top N by cpu)
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .iter()
        .map(|(pid, p)| ProcessInfo {
            pid: pid.as_u32(),
            name: p.name().to_string(),
            cpu_usage: p.cpu_usage(),
            mem_bytes: p.memory(), // adjust if you use virtual_memory() earlier
        })
        .collect();
    procs.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal));
    procs.truncate(30);

    // GPU metrics (never panic)
    let gpus = match crate::gpu::collect_all_gpus() {
        Ok(v) if !v.is_empty() => Some(v),
        Ok(_) => None,
        Err(e) => {
            warn!("gpu collection failed: {e}");
            None
        }
    };

    Metrics {
        cpu_total,
        cpu_per_core,
        mem_total,
        mem_used,
        swap_total,
        swap_used,
        process_count: sys.processes().len(),
        hostname,
        cpu_temp_c,
        disks,
        networks,
        top_processes: procs,
        gpus,
    }
}

// Pick the hottest CPU-like sensor (labels vary by platform)
pub fn best_cpu_temp(components: &Components) -> Option<f32> {
    components
        .iter()
        .filter(|c| {
            let label = c.label().to_lowercase();
            label.contains("cpu")
                || label.contains("package")
                || label.contains("tctl")
                || label.contains("tdie")
        })
        .filter_map(|c| c.temperature())
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}
