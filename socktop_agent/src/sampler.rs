//! Background sampler: periodically collects metrics and updates a JSON cache,
//! so WS replies are just a read of the cached string.

use crate::metrics::collect_metrics;
use crate::state::AppState;
//use serde_json::to_string;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration, MissedTickBehavior};

pub fn spawn_sampler(state: AppState, period: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let idle_period = Duration::from_secs(10);
        loop {
            let active = state
                .client_count
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0;
            let mut ticker = interval(if active { period } else { idle_period });
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            ticker.tick().await;

            if !active {
                tokio::select! {
                    _ = ticker.tick() => {},
                    _ = state.wake_sampler.notified() => continue,
                }
            }

            if let Ok(json) = async {
                let m = collect_metrics(&state).await;
                serde_json::to_string(&m)
            }
            .await
            {
                *state.last_json.write().await = json;
            }
        }
    })
}
