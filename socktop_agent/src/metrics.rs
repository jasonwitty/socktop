//! Metrics collection using sysinfo. Keeps sysinfo handles in AppState to
//! avoid repeated allocations and allow efficient refreshes.

use crate::state::AppState;
use crate::types::{DiskInfo, Metrics, NetworkInfo, ProcessInfo};
use sysinfo::{Components, System};

pub async fn collect_metrics(state: &AppState) -> Metrics {
    // System (CPU/mem/proc)
    let mut sys = state.sys.lock().await;
    // Simple and safe — can be replaced by more granular refresh if desired:
    // sys.refresh_cpu(); sys.refresh_memory(); sys.refresh_processes_specifics(...);
    //sys.refresh_all();
    //refresh all was found to use 2X CPU rather than individual refreshes
    sys.refresh_cpu_all();
    sys.refresh_memory();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());

    // Temps via a persistent Components handle
    let mut components = state.components.lock().await;
    components.refresh(true);
    let cpu_temp_c = best_cpu_temp(&components);

    // Disks via a persistent Disks handle
    let mut disks_struct = state.disks.lock().await;
    disks_struct.refresh(true);
    // Filter anything with available == 0 (e.g., overlay/virtual)
    let disks: Vec<DiskInfo> = disks_struct
        .list()
        .iter()
        .filter(|d| d.available_space() > 0)
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().to_string(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();

    // Networks: use a persistent Networks + rolling totals
    let mut nets = state.nets.lock().await;
    nets.refresh(true);
    let mut totals = state.net_totals.lock().await;
    let mut networks: Vec<NetworkInfo> = Vec::new();
    for (name, data) in nets.iter() {
        // sysinfo: received()/transmitted() are deltas since last refresh
        let delta_rx = data.received();
        let delta_tx = data.transmitted();

        let entry = totals.entry(name.clone()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(delta_rx);
        entry.1 = entry.1.saturating_add(delta_tx);

        networks.push(NetworkInfo {
            name: name.clone(),
            received: entry.0,
            transmitted: entry.1,
        });
    }

    // Normalize process CPU to 0..100 across all cores
    let n_cpus = sys.cpus().len().max(1) as f32;

    // Build process list
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_usage: (p.cpu_usage() / n_cpus).min(100.0),
            mem_bytes: p.memory(),
        })
        .collect();

    // Partial select: get the top 20 by CPU without fully sorting the vector
    const TOP_N: usize = 20;
    if procs.len() > TOP_N {
        // nth index is TOP_N-1 (0-based)
        let nth = TOP_N - 1;
        procs.select_nth_unstable_by(nth, |a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        procs.truncate(TOP_N);
        // Order those 20 nicely for display
        procs.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        procs.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    Metrics {
        cpu_total: sys.global_cpu_usage(),
        cpu_per_core: sys.cpus().iter().map(|c| c.cpu_usage()).collect(),
        mem_total: sys.total_memory(),
        mem_used: sys.used_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        process_count: sys.processes().len(),
        hostname,
        cpu_temp_c,
        disks,
        networks,
        top_processes: procs,
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
