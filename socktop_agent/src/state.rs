//! Shared agent state: sysinfo handles and hot JSON cache.

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use sysinfo::System;
use tokio::sync::{Mutex, Notify, RwLock};

pub type SharedSystem = Arc<Mutex<System>>;

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
}
