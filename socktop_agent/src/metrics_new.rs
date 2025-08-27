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
            kind
        );
        sys.refresh_cpu_usage();

        let total_count = sys.processes().len();
        
        // Reuse allocations via process cache
        let mut proc_cache = state.proc_cache.lock().await;
        proc_cache.reusable_vec.clear();
        
        // Filter and collect processes with meaningful CPU usage
        for p in sys.processes().values() {
            let raw = p.cpu_usage();
            if raw > 0.1 { // Skip negligible CPU users
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
            proc_cache.names.retain(|pid, _| 
                sys.processes().contains_key(&sysinfo::Pid::from_u32(*pid))
            );
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
