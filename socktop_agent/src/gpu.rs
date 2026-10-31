// gpu.rs

use serde::Serialize;

/// One graphics card, as reported on the wire.
///
/// The first four fields are what every agent has always sent and are never
/// optional — clients draw a utilisation + VRAM pair from them alone. Everything
/// after that is additive and simply absent when the driver will not answer:
/// an older driver, a card without the sensor, or a vendor path that cannot
/// read it. `skip_serializing_if` keeps a sensor-less payload byte-identical to
/// an older agent's, so a new agent talking to an old client cannot introduce a
/// field the client has never parsed.
#[derive(Debug, Clone, Serialize)]
pub struct GpuMetrics {
    pub name: String,
    pub utilization_gpu_pct: u32, // 0..100
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,

    /// GPU die temperature in Celsius (`NVML_TEMPERATURE_GPU`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    /// Die temperature at which the card starts throttling (`..._SLOWDOWN`).
    /// Static per card; the client colours its temperature readout against this
    /// instead of guessing a constant that is wrong for another model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_slowdown_c: Option<f32>,
    /// VRAM temperature at which the driver starts a software slowdown
    /// (`..._MEM_MAX`). NVML exposes that *limit* but not a live VRAM reading,
    /// so this is context rather than a gauge — on GDDR6X cards it is the limit
    /// that throttles inference long before the die sensor moves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_mem_max_c: Option<f32>,
    /// Current board power draw in watts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_w: Option<f32>,
    /// Currently enforced power limit in watts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_limit_w: Option<f32>,
    /// Fan 0 speed as a percentage. Absent on passively cooled cards (and cards
    /// whose fans have stopped, where NVML returns an error rather than 0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_pct: Option<u32>,
    /// NVML device index. The client keeps this so a selected card stays the
    /// same physical card between refreshes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvml_index: Option<u32>,
    /// Stable per-card UUID, the strongest identity available for de-duplicating
    /// the primary card against the enumerated list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

/// Collect metrics for every visible GPU, primary card first. `None` when there
/// is no usable GPU.
///
/// Runs on a dedicated worker thread (see `worker`): gfxinfo's handle holds
/// an `Rc<Nvml>` (not `Send`), and *creating* it runs a full NVML library
/// init — ~20ms of blocking work that used to execute on the async runtime
/// for every collection. The worker owns one handle for the process lifetime,
/// so steady-state collection is just NVML queries. Measured on an RTX 5080
/// box, re-initing per collect was ~80% of the agent's entire active CPU.
///
/// The individual NVML *queries* are cheap by comparison (tens of microseconds
/// each), so reading eight of them per card per pass costs nothing worth
/// caching around; the caller's TTL cell is what bounds the rate.
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
        /// Direct NVML with an explicit versioned soname. Debian & friends ship
        /// only libnvidia-ml.so.1 (the unversioned symlink lives in the dev
        /// package), so gfxinfo's default dlopen fails there even though the
        /// driver is fully functional.
        ///
        /// Preferred over gfxinfo because it is the only path that can
        /// enumerate more than one card, and the only one that answers for
        /// temperatures, power and fans. gfxinfo 0.1.2 exposes `active_gpu()`
        /// and nothing else.
        Nvml(Box<nvml_wrapper::Nvml>),
        /// gfxinfo's own detection (AMD sysfs, NVIDIA via unversioned NVML, and
        /// whatever it does on Windows). A single card, no sensor detail — the
        /// fallback for hosts where NVML is unavailable.
        Gfx(Box<dyn gfxinfo::Gpu>),
    }

    fn probe() -> Option<Handle> {
        // NVML's library name is platform-specific. Debian & friends ship only
        // libnvidia-ml.so.1 (the unversioned symlink lives in the dev package), so
        // gfxinfo's default dlopen fails there even though the driver is fully
        // functional; on Windows the driver ships nvml.dll in System32.
        #[cfg(windows)]
        let lib = std::ffi::OsStr::new("nvml.dll");
        #[cfg(not(windows))]
        let lib = std::ffi::OsStr::new("libnvidia-ml.so.1");

        nvml_wrapper::Nvml::builder()
            .lib_path(lib)
            .init()
            .ok()
            .map(|nvml| Handle::Nvml(Box::new(nvml)))
            .or_else(|| gfxinfo::active_gpu().ok().map(Handle::Gfx))
    }

    /// NVML answers per card, so every query is taken on its own: a driver that
    /// refuses one (fan tachometer on a passive card, threshold on an old
    /// driver) must not cost the client the utilisation it did answer.
    fn from_nvml(device: &nvml_wrapper::Device, index: u32) -> GpuMetrics {
        use nvml_wrapper::enum_wrappers::device::{TemperatureSensor, TemperatureThreshold};

        let (mem_used, mem_total) = match device.memory_info() {
            Ok(m) => (m.used, m.total),
            Err(_) => (0, 0),
        };
        let power_w = device.power_usage().ok().map(|mw| mw as f32 / 1000.0);
        // `enforced_power_limit` is what the card is actually held to (a miner
        // or an LLM host that has been power-capped reports the cap, not the
        // out-of-the-box default); fall back to the configured limit.
        let power_limit_w = device
            .enforced_power_limit()
            .or_else(|_| device.power_management_limit())
            .ok()
            .map(|mw| mw as f32 / 1000.0);

        GpuMetrics {
            name: device.name().unwrap_or_else(|_| "NVIDIA GPU".into()),
            utilization_gpu_pct: device
                .utilization_rates()
                .map(|u| u.gpu.clamp(0, 100))
                .unwrap_or(0),
            mem_used_bytes: mem_used,
            mem_total_bytes: mem_total,
            temp_c: device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(|t| t as f32),
            temp_slowdown_c: device
                .temperature_threshold(TemperatureThreshold::Slowdown)
                .ok()
                .map(|t| t as f32)
                .filter(|t| *t > 0.0),
            temp_mem_max_c: device
                .temperature_threshold(TemperatureThreshold::MemoryMax)
                .ok()
                .map(|t| t as f32)
                .filter(|t| *t > 0.0),
            power_w,
            power_limit_w,
            fan_pct: device.fan_speed(0).ok().map(|f| f.min(100)),
            nvml_index: Some(index),
            uuid: device.uuid().ok(),
        }
    }

    fn collect_from(handle: &Handle) -> Option<Vec<GpuMetrics>> {
        match handle {
            Handle::Nvml(nvml) => {
                let count = nvml.device_count().ok()?;
                let mut out = Vec::with_capacity(count as usize);
                for i in 0..count {
                    if let Ok(device) = nvml.device_by_index(i) {
                        out.push(from_nvml(&device, i));
                    }
                }
                if out.is_empty() { None } else { Some(out) }
            }
            Handle::Gfx(gpu) => {
                let info = gpu.info();
                Some(vec![GpuMetrics {
                    name: gpu.model().to_string(),
                    utilization_gpu_pct: info.load_pct().clamp(0, 100),
                    mem_used_bytes: info.used_vram(),
                    mem_total_bytes: info.total_vram(),
                    // Deliberately no temperatures from gfxinfo: its NVIDIA path
                    // returns millidegrees Celsius and its AMD path returns the
                    // raw libdrm sensor value, which are not the same scale, and
                    // neither is documented well enough to guess from. A card
                    // reachable only through gfxinfo shows `—` for temps rather
                    // than a number that might be wrong by 1000x.
                    temp_c: None,
                    temp_slowdown_c: None,
                    temp_mem_max_c: None,
                    power_w: None,
                    power_limit_w: None,
                    fan_pct: None,
                    nvml_index: None,
                    uuid: None,
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

#[cfg(test)]
mod tests {
    #[cfg(feature = "gpu")]
    #[test]
    fn an_nvml_card_with_no_optional_sensors_serializes_like_an_old_agent() {
        // The wire compatibility of the whole feature rests on this: a driver
        // that answers nothing optional must produce exactly the JSON shape
        // agents have always sent.
        let g = super::GpuMetrics {
            name: "NVIDIA GeForce RTX 3090".into(),
            utilization_gpu_pct: 0,
            mem_used_bytes: 0,
            mem_total_bytes: 24_000_000_000,
            temp_c: None,
            temp_slowdown_c: None,
            temp_mem_max_c: None,
            power_w: None,
            power_limit_w: None,
            fan_pct: None,
            nvml_index: None,
            uuid: None,
        };
        let json = serde_json::to_string(&g).expect("serialize");
        for absent in [
            "temp_c",
            "temp_slowdown_c",
            "temp_mem_max_c",
            "power_w",
            "power_limit_w",
            "fan_pct",
            "nvml_index",
            "uuid",
        ] {
            assert!(!json.contains(absent), "{absent} leaked: {json}");
        }
        for present in [
            "name",
            "utilization_gpu_pct",
            "mem_used_bytes",
            "mem_total_bytes",
        ] {
            assert!(json.contains(present), "{present} missing: {json}");
        }
    }
}
