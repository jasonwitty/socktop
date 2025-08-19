use socktop::ws::{connect, request_metrics, request_processes};

// Integration probe: only runs when SOCKTOP_WS is set to an agent WebSocket URL.
// Example: SOCKTOP_WS=ws://127.0.0.1:3000/ws cargo test -p socktop --test ws_probe -- --nocapture
#[tokio::test]
async fn probe_ws_endpoints() {
    // Gate the test to avoid CI failures when no agent is running.
    let url = match std::env::var("SOCKTOP_WS") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!(
                "skipping ws_probe: set SOCKTOP_WS=ws://host:port/ws to run this integration test"
            );
            return;
        }
    };

    let mut ws = connect(&url).await.expect("connect ws");

    // Should get fast metrics quickly
    let m = request_metrics(&mut ws).await;
    assert!(m.is_some(), "expected Metrics payload within timeout");

    // Processes may be gzipped and a bit slower, but should arrive
    let p = request_processes(&mut ws).await;
    assert!(p.is_some(), "expected Processes payload within timeout");
}
