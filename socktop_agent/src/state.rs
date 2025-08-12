//! Shared agent state: sysinfo handles and hot JSON cache.

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use sysinfo::{Components, Disks, Networks, System};
use tokio::sync::{Mutex, Notify, RwLock};

pub type SharedSystem = Arc<Mutex<System>>;
pub type SharedComponents = Arc<Mutex<Components>>;
pub type SharedDisks = Arc<Mutex<Disks>>;
pub type SharedNetworks = Arc<Mutex<Networks>>;

#[derive(Clone)]
pub struct AppState {
    // Persistent sysinfo handles
    pub sys: SharedSystem,

    // Last serialized JSON snapshot for fast WS responses
    pub last_json: Arc<RwLock<String>>,

    // Adaptive sampling controls
    pub client_count: Arc<AtomicUsize>,
    pub wake_sampler: Arc<Notify>,
    pub auth_token: Option<String>,

    // Cached containers (enumerated once; refreshed per tick)
    pub components: SharedComponents,
    pub disks: SharedDisks,
    pub networks: SharedNetworks,
}

impl AppState {

    #[allow(dead_code)]
    pub fn new() -> Self {
        let sys = System::new(); // targeted refreshes per tick
        let components = Components::new_with_refreshed_list(); // enumerate once
        let disks = Disks::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();
        Self {
            sys: Arc::new(Mutex::new(sys)),
            components: Arc::new(Mutex::new(components)),
            disks: Arc::new(Mutex::new(disks)),
            networks: Arc::new(Mutex::new(networks)),
            last_json: Arc::new(RwLock::new(String::new())),
            client_count: Arc::new(AtomicUsize::new(0)),
            wake_sampler: Arc::new(Notify::new()),
            auth_token: std::env::var("SOCKTOP_TOKEN").ok().filter(|s| !s.is_empty()),
        }
    }
}
