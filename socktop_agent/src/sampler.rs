//! Background sampler: periodically collects metrics and updates precompressed caches,
//! so WS replies just read and send cached bytes.

use crate::state::AppState;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

// 500ms: fast path (cpu/mem/net/temp/gpu)
pub fn spawn_sampler(_state: AppState, _period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        // no-op background sampler (request-driven collection elsewhere)
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    })
}

// 2s: processes top-k
pub fn spawn_process_sampler(_state: AppState, _period: Duration, _top_k: usize) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    })
}

// 5s: disks
pub fn spawn_disks_sampler(_state: AppState, _period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    })
}
