//! Metrics collection using sysinfo for socktop_agent.

use crate::gpu::collect_all_gpus;
use crate::state::AppState;
use crate::types::{DiskInfo, Metrics, NetworkInfo, ProcessInfo};

use std::cmp::Ordering;
use sysinfo::{ProcessesToUpdate, System};
use tracing::warn;

pub async fn collect_metrics(state: &AppState) -> Metrics {
    let mut sys = state.sys.lock().await;

    // Targeted refresh: CPU/mem/processes only
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(ProcessesToUpdate::All, true);
    })) {
        warn!("sysinfo selective refresh panicked: {e:?}");
    }

    // Hostname
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());

    // CPU usage
    let cpu_total = sys.global_cpu_usage();
    let cpu_per_core: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();

    // Memory / swap
    let mem_total = sys.total_memory();
    let mem_used = mem_total.saturating_sub(sys.available_memory());
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    drop(sys); // release quickly before touching other locks

    // Components (cached): just refresh temps
    let cpu_temp_c = {
        let mut components = state.components.lock().await;
        components.refresh(true);
        components.iter().find_map(|c| {
            let l = c.label().to_ascii_lowercase();
            if l.contains("cpu") || l.contains("package") || l.contains("tctl") || l.contains("tdie") {
                c.temperature()
            } else {
                None
            }
        })
    };

    // Disks (cached): refresh sizes/usage, reuse enumeration
    let disks: Vec<DiskInfo> = {
        let mut disks_list = state.disks.lock().await;
        disks_list.refresh(true);
        disks_list
            .iter()
            .map(|d| DiskInfo {
                name: d.name().to_string_lossy().into_owned(),
                total: d.total_space(),
                available: d.available_space(),
            })
            .collect()
    };

    // Networks (cached): refresh counters
    let networks: Vec<NetworkInfo> = {
        let mut nets = state.networks.lock().await;
        nets.refresh(true);
        nets.iter()
            .map(|(name, data)| NetworkInfo {
                name: name.to_string(),
                received: data.total_received(),
                transmitted: data.total_transmitted()
            })
            .collect()
    };

    // Processes: only collect fields we use (pid, name, cpu, mem), keep top K efficiently
    const TOP_K: usize = 30;
    let mut procs: Vec<ProcessInfo> = {
        let sys = state.sys.lock().await; // re-lock briefly to read processes
        sys.processes()
            .values()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: p.cpu_usage(),
                mem_bytes: p.memory()
            })
            .collect()
    };

    if procs.len() > TOP_K {
        procs.select_nth_unstable_by(TOP_K, |a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(Ordering::Equal)
        });
        procs.truncate(TOP_K);
    }
    procs.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(Ordering::Equal)
    });

    let process_count = {
        let sys = state.sys.lock().await;
        sys.processes().len()
    };

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
        process_count,
        hostname,
        cpu_temp_c,
        disks,
        networks,
        top_processes: procs,
        gpus
    }
}