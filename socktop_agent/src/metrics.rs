//! Metrics collection using sysinfo for socktop_agent.

use crate::gpu::collect_all_gpus;
use crate::state::AppState;
use crate::types::{DiskInfo, Metrics, NetworkInfo, ProcessInfo};

use sysinfo::{Components, Disks, Networks, System};
use tracing::warn;

pub async fn collect_metrics(state: &AppState) -> Metrics {
    let mut sys = state.sys.lock().await;

    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sys.refresh_all();
    })) {
        warn!("sysinfo refresh panicked: {e:?}");
    }

    // Hostname (associated fn on System in 0.37)
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());

    // CPU usage
    let cpu_total = sys.global_cpu_usage();
    let cpu_per_core: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();

    // Memory / swap
    let mem_total = sys.total_memory();
    let mem_used = mem_total.saturating_sub(sys.available_memory());
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    // Temperature (via Components container)
    let components = Components::new_with_refreshed_list();
    let cpu_temp_c = components.iter().find_map(|c| {
        let l = c.label().to_ascii_lowercase();
        if l.contains("cpu") || l.contains("package") || l.contains("tctl") || l.contains("tdie") {
            c.temperature()
        } else {
            None
        }
    });

    // Disks (via Disks container)
    let disks_list = Disks::new_with_refreshed_list();
    let disks: Vec<DiskInfo> = disks_list
        .iter()
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();

    // Networks (via Networks container) – include interface name
    let nets = Networks::new_with_refreshed_list();
    let networks: Vec<NetworkInfo> = nets
        .iter()
        .map(|(name, data)| NetworkInfo {
            name: name.to_string(),
            received: data.total_received(),
            transmitted: data.total_transmitted(),
        })
        .collect();

    // Processes (top N by CPU)
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
    procs.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal));
    procs.truncate(30);

    // GPU(s)
    let gpus = match collect_all_gpus() {
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