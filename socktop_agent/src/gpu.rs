// gpu.rs

#[derive(Debug, Clone, serde::Serialize)]
pub struct GpuMetrics {
    pub name: String,
    pub utilization_gpu_pct: u32, // 0..100
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
}

/// Collect metrics for the active GPU. `None` when there is no usable GPU.
///
/// Runs on a dedicated worker thread (see `worker`): gfxinfo's handle holds
/// an `Rc<Nvml>` (not `Send`), and *creating* it runs a full NVML library
/// init — ~20ms of blocking work that used to execute on the async runtime
/// for every collection. The worker owns one handle for the process lifetime,
/// so steady-state collection is just NVML queries. Measured on an RTX 5080
/// box, re-initing per collect was ~80% of the agent's entire active CPU.
#[cfg(feature = "gpu")]
pub async fn collect_all_gpus() -> Option<Vec<GpuMetrics>> {
    worker::collect().await
}

#[cfg(not(feature = "gpu"))]
pub async fn collect_all_gpus() -> Option<Vec<GpuMetrics>> {
    None
}

#[cfg(feature = "gpu")]
mod worker {
    use super::GpuMetrics;
    use once_cell::sync::OnceCell;
    use std::sync::mpsc;

    type Reply = tokio::sync::oneshot::Sender<Option<Vec<GpuMetrics>>>;
    static TX: OnceCell<mpsc::Sender<Reply>> = OnceCell::new();

    pub async fn collect() -> Option<Vec<GpuMetrics>> {
        let tx = TX.get_or_init(spawn);
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(reply_tx).ok()?;
        reply_rx.await.ok().flatten()
    }

    fn spawn() -> mpsc::Sender<Reply> {
        let (tx, rx) = mpsc::channel::<Reply>();
        std::thread::Builder::new()
            .name("socktop-gpu".into())
            .spawn(move || run(rx))
            .expect("spawn gpu worker thread");
        tx
    }

    enum Handle {
        /// gfxinfo's own detection (AMD sysfs, NVIDIA via unversioned NVML).
        Gfx(Box<dyn gfxinfo::Gpu>),
        /// Direct NVML with an explicit versioned soname. Debian & friends
        /// ship only libnvidia-ml.so.1 (the unversioned symlink lives in the
        /// dev package), so gfxinfo's default dlopen fails there even though
        /// the driver is fully functional.
        Nvml(Box<nvml_wrapper::Nvml>),
    }

    fn probe() -> Option<Handle> {
        if let Ok(g) = gfxinfo::active_gpu() {
            return Some(Handle::Gfx(g));
        }
        nvml_wrapper::Nvml::builder()
            .lib_path(std::ffi::OsStr::new("libnvidia-ml.so.1"))
            .init()
            .ok()
            .map(|nvml| Handle::Nvml(Box::new(nvml)))
    }

    fn collect_from(handle: &Handle) -> Option<Vec<GpuMetrics>> {
        match handle {
            Handle::Gfx(gpu) => {
                let info = gpu.info();
                Some(vec![GpuMetrics {
                    name: gpu.model().to_string(),
                    utilization_gpu_pct: info.load_pct().clamp(0, 100),
                    mem_used_bytes: info.used_vram(),
                    mem_total_bytes: info.total_vram(),
                }])
            }
            Handle::Nvml(nvml) => {
                let device = nvml.device_by_index(0).ok()?;
                let mem = device.memory_info().ok()?;
                Some(vec![GpuMetrics {
                    name: device.name().unwrap_or_else(|_| "NVIDIA GPU".into()),
                    utilization_gpu_pct: device
                        .utilization_rates()
                        .map(|u| u.gpu.clamp(0, 100))
                        .unwrap_or(0),
                    mem_used_bytes: mem.used,
                    mem_total_bytes: mem.total,
                }])
            }
        }
    }

    fn run(rx: mpsc::Receiver<Reply>) {
        let mut handle: Option<Handle> = None;
        // Probing failed: remember and answer None without re-initing the GPU
        // stack per request. The agent's negative cache stops asking anyway.
        let mut probe_failed = false;
        while let Ok(reply) = rx.recv() {
            if handle.is_none() && !probe_failed {
                handle = probe();
                probe_failed = handle.is_none();
            }
            let out = handle.as_ref().and_then(collect_from);
            // A live GPU cannot report 0 total VRAM; zeros mean the session
            // died (e.g. driver reload). Drop the handle so the next request
            // re-probes.
            if let Some(v) = &out
                && !v.is_empty()
                && v.iter().all(|g| g.mem_total_bytes == 0)
            {
                handle = None;
            }
            let _ = reply.send(out.filter(|v| !v.is_empty()));
        }
    }
}
