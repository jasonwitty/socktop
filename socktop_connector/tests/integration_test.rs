use socktop_connector::{
    AgentRequest, AgentResponse, connect_to_socktop_agent, connect_to_socktop_agent_with_tls,
};

// Integration probe: only runs when SOCKTOP_WS is set to an agent WebSocket URL.
// Example: SOCKTOP_WS=ws://127.0.0.1:3000/ws cargo test -p socktop_connector --test integration_test -- --nocapture
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

    // Optional pinned CA for WSS/self-signed setups
    let tls_ca = std::env::var("SOCKTOP_TLS_CA").ok();

    let mut connector = if let Some(ca_path) = tls_ca {
        connect_to_socktop_agent_with_tls(&url, ca_path, true)
            .await
            .expect("connect ws with TLS")
    } else {
        connect_to_socktop_agent(&url).await.expect("connect ws")
    };

    // Should get fast metrics quickly
    let response = connector.request(AgentRequest::Metrics).await;
    assert!(response.is_ok(), "expected Metrics payload within timeout");
    if let Ok(AgentResponse::Metrics(_)) = response {
        // Success
    } else {
        panic!("expected Metrics response");
    }

    // Processes may be gzipped and a bit slower, but should arrive
    let response = connector.request(AgentRequest::Processes).await;
    assert!(
        response.is_ok(),
        "expected Processes payload within timeout"
    );
    if let Ok(AgentResponse::Processes(_)) = response {
        // Success
    } else {
        panic!("expected Processes response");
    }
}

/// The client draws its memory bar as three stacked segments — used, cache
/// (`available - free`) and free — so the numbers arriving over the wire have
/// to add up. CI already starts a real agent and runs this crate's tests
/// against it on Linux and Windows, which is the only place that invariant can
/// be checked against an actual OS rather than a fixture.
#[tokio::test]
async fn probe_memory_figures_add_up() {
    let url = match std::env::var("SOCKTOP_WS") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("skipping probe_memory_figures_add_up: set SOCKTOP_WS to run this test");
            return;
        }
    };

    let tls_ca = std::env::var("SOCKTOP_TLS_CA").ok();
    let mut connector = if let Some(ca_path) = tls_ca {
        connect_to_socktop_agent_with_tls(&url, ca_path, true)
            .await
            .expect("connect ws with TLS")
    } else {
        connect_to_socktop_agent(&url).await.expect("connect ws")
    };

    let response = connector
        .request(AgentRequest::Metrics)
        .await
        .expect("expected Metrics payload within timeout");
    let m = match response {
        AgentResponse::Metrics(m) => m,
        _ => panic!("expected Metrics response"),
    };

    assert!(m.mem_total > 0, "agent reported zero total memory");
    if m.mem_available == 0 {
        eprintln!("agent does not report mem_available (older agent); nothing to check");
        return;
    }

    assert!(
        m.mem_available >= m.mem_free,
        "available ({} MB) is below free ({} MB)",
        m.mem_available / 1024,
        m.mem_free / 1024
    );
    assert!(
        m.mem_used <= m.mem_total,
        "used ({} MB) exceeds total ({} MB)",
        m.mem_used / 1024,
        m.mem_total / 1024
    );

    let cache = m.mem_available - m.mem_free;
    let sum = m.mem_used + cache + m.mem_free;
    // Platforms read their counters at slightly different instants, so allow
    // a MiB of drift between the two sides of the identity.
    let drift = sum.abs_diff(m.mem_total);
    assert!(
        drift <= 1 << 20,
        "used {} + cache {} + free {} = {} bytes, {} bytes off total {}",
        m.mem_used,
        cache,
        m.mem_free,
        sum,
        drift,
        m.mem_total
    );
    eprintln!(
        "memory split ok: used {} MB, cache {} MB, free {} MB (total {} MB)",
        m.mem_used / 1024 / 1024,
        cache / 1024 / 1024,
        m.mem_free / 1024 / 1024,
        m.mem_total / 1024 / 1024
    );
}
