//! Shared agent state: sysinfo handles and hot JSON cache.

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use sysinfo::{Components, Disks, Networks, System};
use tokio::sync::Mutex;

pub type SharedSystem = Arc<Mutex<System>>;
pub type SharedComponents = Arc<Mutex<Components>>;
pub type SharedDisks = Arc<Mutex<Disks>>;
pub type SharedNetworks = Arc<Mutex<Networks>>;

#[derive(Default)]
pub struct ProcCpuTracker {
    pub last_total: u64,
    pub last_per_pid: HashMap<u32, u64>,
}

#[derive(Clone)]
pub struct AppState {
    pub sys: SharedSystem,
    pub components: SharedComponents,
    pub disks: SharedDisks,
    pub networks: SharedNetworks,

    // For correct per-process CPU% using /proc deltas
    pub proc_cpu: Arc<Mutex<ProcCpuTracker>>,

    // Connection tracking (to allow future idle sleeps if desired)
    pub client_count: Arc<AtomicUsize>,

    pub auth_token: Option<String>,
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
            proc_cpu: Arc::new(Mutex::new(ProcCpuTracker::default())),
            client_count: Arc::new(AtomicUsize::new(0)),
            auth_token: std::env::var("SOCKTOP_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}
