//! Shared agent state: sysinfo handles and hot JSON cache.

use std::{collections::HashMap, sync::Arc};
use std::sync::atomic::AtomicUsize;
use sysinfo::{Components, Disks, Networks, System};
use tokio::sync::{Mutex, RwLock, Notify};

pub type SharedSystem = Arc<Mutex<System>>;
pub type SharedNetworks = Arc<Mutex<Networks>>;
pub type SharedTotals = Arc<Mutex<HashMap<String, (u64, u64)>>>;
pub type SharedComponents = Arc<Mutex<Components>>;
pub type SharedDisks = Arc<Mutex<Disks>>;

#[derive(Clone)]
pub struct AppState {
    // Persistent sysinfo handles
    pub sys: SharedSystem,
    pub nets: SharedNetworks,
    pub net_totals: SharedTotals, // iface -> (rx_total, tx_total)
    pub components: SharedComponents,
    pub disks: SharedDisks,

    // Last serialized JSON snapshot for fast WS responses
    pub last_json: Arc<RwLock<String>>,

    // Adaptive sampling controls
    pub client_count: Arc<AtomicUsize>,
    pub wake_sampler: Arc<Notify>,
    pub auth_token: Option<String>,
}