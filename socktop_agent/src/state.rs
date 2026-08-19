//! Shared agent state: sysinfo handles and hot JSON cache.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};
use sysinfo::{Components, Disks, Networks, System};
use tokio::sync::Mutex;

pub type SharedSystem = Arc<Mutex<System>>;
pub type SharedComponents = Arc<Mutex<Components>>;
pub type SharedDisks = Arc<Mutex<Disks>>;
pub type SharedNetworks = Arc<Mutex<Networks>>;

#[cfg(target_os = "linux")]
#[derive(Default)]
pub struct ProcCpuTracker {
    pub last_total: u64,
    pub last_per_pid: HashMap<u32, u64>,
    /// PID → process name cache. Mirrors the non-Linux `ProcessCache.names`.
    /// On a Pi with ~150-300 mostly-stable processes this avoids re-allocating
    /// the same `String`s on every processes poll (~once per 1.5s).
    pub names: HashMap<u32, String>,
}

#[cfg(not(target_os = "linux"))]
pub struct ProcessCache {
    pub names: HashMap<u32, String>,
    pub reusable_vec: Vec<crate::types::ProcessInfo>,
}

#[cfg(not(target_os = "linux"))]
impl Default for ProcessCache {
    fn default() -> Self {
        Self {
            names: HashMap::with_capacity(1000), // Pre-allocate for typical modern system process count
            reusable_vec: Vec::with_capacity(1000),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub sys: SharedSystem,
    pub components: SharedComponents,
    pub disks: SharedDisks,
    pub networks: SharedNetworks,
    pub hostname: String,

    // For correct per-process CPU% using /proc deltas (Linux only path uses this tracker)
    #[cfg(target_os = "linux")]
    pub proc_cpu: Arc<Mutex<ProcCpuTracker>>,

    // Process name caching and vector reuse for non-Linux to reduce allocations
    #[cfg(not(target_os = "linux"))]
    pub proc_cache: Arc<Mutex<ProcessCache>>,

    // Connection tracking (to allow future idle sleeps if desired)
    pub client_count: Arc<AtomicUsize>,

    pub auth_token: Option<String>,
    // GPU negative cache (probe once). gpu_checked=true after first attempt; gpu_present reflects result.
    pub gpu_checked: Arc<AtomicBool>,
    pub gpu_present: Arc<AtomicBool>,

    // Lightweight on-demand caches (TTL based) to cap CPU under bursty polling.
    pub cache_metrics: Arc<Mutex<CacheEntry<crate::types::Metrics>>>,
    pub cache_disks: Arc<Mutex<CacheEntry<Vec<crate::types::DiskInfo>>>>,
    pub cache_processes: Arc<Mutex<CacheEntry<crate::types::ProcessesPayload>>>,

    // Process detail caches (per-PID)
    pub cache_process_metrics:
        Arc<Mutex<HashMap<u32, CacheEntry<crate::types::ProcessMetricsResponse>>>>,
    pub cache_journal_entries: Arc<Mutex<HashMap<u32, CacheEntry<crate::types::JournalResponse>>>>,
}

/// TTL-gated value behind a std Mutex, for `static` caches on hot paths.
/// Replaces the hand-rolled TempCache/GpuCache/refresh-timestamp statics
/// that each reimplemented the same at/value pair.
pub struct TtlCell<T> {
    inner: std::sync::Mutex<CacheEntry<T>>,
}

impl<T: Clone> Default for TtlCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> TtlCell<T> {
    pub const fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(CacheEntry::new()),
        }
    }
    /// The stored value, only while fresh. Poisoned lock reads as a miss.
    pub fn get_fresh(&self, ttl: Duration) -> Option<T> {
        let g = self.inner.lock().ok()?;
        if g.is_fresh(ttl) {
            g.value.clone()
        } else {
            None
        }
    }
    pub fn set(&self, v: T) {
        if let Ok(mut g) = self.inner.lock() {
            g.set(v);
        }
    }
    /// True exactly once per TTL window: restamps and tells the caller to do
    /// the refresh. Atomic check-and-stamp so concurrent callers don't both
    /// refresh.
    pub fn claim_stale(&self, ttl: Duration) -> bool {
        let Ok(mut g) = self.inner.lock() else {
            return false;
        };
        if g.at.is_none_or(|t| t.elapsed() >= ttl) {
            g.at = Some(Instant::now());
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug)]
pub struct CacheEntry<T> {
    pub at: Option<Instant>,
    pub value: Option<T>,
}

impl<T> Default for CacheEntry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> CacheEntry<T> {
    pub const fn new() -> Self {
        Self {
            at: None,
            value: None,
        }
    }
    pub fn is_fresh(&self, ttl: Duration) -> bool {
        self.at.is_some_and(|t| t.elapsed() < ttl) && self.value.is_some()
    }
    pub fn set(&mut self, v: T) {
        self.value = Some(v);
        self.at = Some(Instant::now());
    }
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let sys = System::new();
        let components = Components::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();

        Self {
            sys: Arc::new(Mutex::new(sys)),
            components: Arc::new(Mutex::new(components)),
            disks: Arc::new(Mutex::new(disks)),
            networks: Arc::new(Mutex::new(networks)),
            hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
            #[cfg(target_os = "linux")]
            proc_cpu: Arc::new(Mutex::new(ProcCpuTracker::default())),
            #[cfg(not(target_os = "linux"))]
            proc_cache: Arc::new(Mutex::new(ProcessCache::default())),
            client_count: Arc::new(AtomicUsize::new(0)),
            auth_token: std::env::var("SOCKTOP_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            gpu_checked: Arc::new(AtomicBool::new(false)),
            gpu_present: Arc::new(AtomicBool::new(false)),
            cache_metrics: Arc::new(Mutex::new(CacheEntry::new())),
            cache_disks: Arc::new(Mutex::new(CacheEntry::new())),
            cache_processes: Arc::new(Mutex::new(CacheEntry::new())),
            cache_process_metrics: Arc::new(Mutex::new(HashMap::new())),
            cache_journal_entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
