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
#[cfg(all(feature = "gpu", not(target_os = "macos")))]
pub async fn collect_all_gpus() -> Option<Vec<GpuMetrics>> {
    worker::collect().await
}

/// macOS: read the IORegistry directly (see `macos`). The query is a few
/// blocking IOKit calls with no handle to keep alive, so the blocking pool is
/// enough — no dedicated worker.
#[cfg(all(feature = "gpu", target_os = "macos"))]
pub async fn collect_all_gpus() -> Option<Vec<GpuMetrics>> {
    tokio::task::spawn_blocking(macos::collect)
        .await
        .ok()
        .flatten()
}

#[cfg(not(feature = "gpu"))]
pub async fn collect_all_gpus() -> Option<Vec<GpuMetrics>> {
    None
}

#[cfg(all(feature = "gpu", not(target_os = "macos")))]
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

/// GPU metrics from the IORegistry's `IOAccelerator` services.
///
/// Replaces gfxinfo on macOS: gfxinfo 0.1.2 releases the properties dictionary
/// (and a child it only borrows) more than once per query, and CoreFoundation
/// on recent macOS traps on that — killing the whole agent with SIGTRAP on the
/// first metrics request. Here every CoreFoundation / IOKit object has exactly
/// one owner, released by `Drop`.
#[cfg(all(feature = "gpu", target_os = "macos"))]
mod macos {
    use super::GpuMetrics;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::data::CFData;
    use core_foundation::dictionary::{CFDictionary, CFMutableDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use io_kit_sys::types::io_object_t;
    use io_kit_sys::{
        IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperties,
        IOServiceGetMatchingServices, IOServiceMatching, kIOMasterPortDefault,
    };

    /// Owned IOKit object handle (iterator or registry entry).
    struct IoObject(io_object_t);

    impl Drop for IoObject {
        fn drop(&mut self) {
            // SAFETY: we own exactly one reference to this non-zero handle.
            unsafe { IOObjectRelease(self.0) };
        }
    }

    /// Look up `key`, returning a retained, type-erased value.
    fn get(dict: &CFDictionary, key: &'static str) -> Option<CFType> {
        let key = CFString::from_static_string(key);
        let value = dict.find(key.as_CFTypeRef())?;
        if value.is_null() {
            return None;
        }
        // SAFETY: values in a property-list dictionary are CF objects; the
        // dictionary only lends them to us, so retain (get rule).
        Some(unsafe { CFType::wrap_under_get_rule(*value) })
    }

    fn number(dict: &CFDictionary, key: &'static str) -> Option<u64> {
        let n = get(dict, key)?.downcast::<CFNumber>()?.to_i64()?;
        u64::try_from(n).ok()
    }

    /// `model` is a string on Apple Silicon and NUL-terminated bytes on some
    /// Intel/AMD drivers.
    fn model(dict: &CFDictionary) -> Option<String> {
        let value = get(dict, "model")?;
        let name = if let Some(s) = value.downcast::<CFString>() {
            s.to_string()
        } else {
            let data = value.downcast::<CFData>()?;
            String::from_utf8_lossy(data.bytes()).into_owned()
        };
        let name = name.trim_matches(|c: char| c == '\0' || c.is_whitespace());
        (!name.is_empty()).then(|| name.to_string())
    }

    /// Properties of one registry entry, as an owned dictionary.
    fn properties(entry: &IoObject) -> Option<CFDictionary> {
        let mut props: CFMutableDictionaryRef = std::ptr::null_mut();
        // SAFETY: `entry` is a live registry entry and `props` a valid out
        // pointer. On success we own the created dictionary (create rule).
        unsafe {
            if IORegistryEntryCreateCFProperties(entry.0, &mut props, std::ptr::null(), 0) != 0
                || props.is_null()
            {
                return None;
            }
            Some(CFDictionary::wrap_under_create_rule(props as _))
        }
    }

    /// Dedicated GPU memory in use. Apple Silicon has none — GPU memory is
    /// system RAM, already reported as memory — so report 0 there instead of
    /// passing off the driver's system-memory figure as VRAM.
    fn vram_used(stats: &CFDictionary) -> u64 {
        if cfg!(target_arch = "aarch64") {
            0
        } else {
            number(stats, "In use system memory").unwrap_or(0)
        }
    }

    pub fn collect() -> Option<Vec<GpuMetrics>> {
        // SAFETY: IOServiceMatching returns a +1 dictionary which
        // IOServiceGetMatchingServices consumes, success or not.
        let iter = unsafe {
            let matching = IOServiceMatching(c"IOAccelerator".as_ptr());
            if matching.is_null() {
                return None;
            }
            let mut iter: io_object_t = 0;
            if IOServiceGetMatchingServices(kIOMasterPortDefault, matching, &mut iter) != 0
                || iter == 0
            {
                return None;
            }
            IoObject(iter)
        };

        let mut gpus = Vec::new();
        loop {
            // SAFETY: `iter` is a live iterator; a non-zero result is a +1 entry.
            let entry = match unsafe { IOIteratorNext(iter.0) } {
                0 => break,
                e => IoObject(e),
            };
            let Some(props) = properties(&entry) else {
                continue;
            };
            // Accelerators without live statistics (e.g. a powered-down
            // discrete GPU) have nothing to report.
            let Some(stats) =
                get(&props, "PerformanceStatistics").and_then(|v| v.downcast::<CFDictionary>())
            else {
                continue;
            };
            gpus.push(GpuMetrics {
                name: model(&props).unwrap_or_else(|| "GPU".into()),
                utilization_gpu_pct: number(&stats, "Device Utilization %").unwrap_or(0).min(100)
                    as u32,
                mem_used_bytes: vram_used(&stats),
                // The registry exposes no VRAM total; 0 means "unknown / none"
                // and clients show no VRAM readout for it.
                mem_total_bytes: 0,
            });
        }
        (!gpus.is_empty()).then_some(gpus)
    }

    #[cfg(test)]
    mod tests {
        /// Regression: the previous backend double-released CoreFoundation
        /// objects, which aborts the process rather than failing a query.
        /// Repeated collection must survive and stay self-consistent.
        #[test]
        fn repeated_collection_is_stable() {
            for _ in 0..200 {
                if let Some(gpus) = super::collect() {
                    for g in gpus {
                        assert!(!g.name.is_empty());
                        assert!(g.utilization_gpu_pct <= 100);
                    }
                }
            }
        }
    }
}
