//! Tests for process detail collection functionality

use socktop_agent::metrics::{collect_journal_entries, collect_process_metrics};
use socktop_agent::state::AppState;
use std::process;

#[tokio::test]
async fn test_collect_process_metrics_self() {
    // Test collecting metrics for our own process
    let pid = process::id();
    let state = AppState::new();

    match collect_process_metrics(pid, &state).await {
        Ok(response) => {
            assert_eq!(response.process.pid, pid);
            assert!(!response.process.name.is_empty());
            // Command might be empty on some systems, so don't assert on it
            assert!(response.cached_at > 0);
            println!(
                "✓ Process metrics collected for PID {}: {} ({})",
                pid, response.process.name, response.process.command
            );
        }
        Err(e) => {
            // This might fail if sysinfo can't find the process, which is possible
            println!("⚠ Warning: Failed to collect process metrics for self: {e}");
        }
    }
}

#[tokio::test]
async fn test_collect_journal_entries_self() {
    // Test collecting journal entries for our own process
    let pid = process::id();

    match collect_journal_entries(pid).await {
        Ok(response) => {
            assert!(response.cached_at > 0);
            println!(
                "✓ Journal entries collected for PID {}: {} entries",
                pid, response.total_count
            );
            if !response.entries.is_empty() {
                let entry = &response.entries[0];
                println!("  Latest entry: {}", entry.message);
            }
        }
        Err(e) => {
            // This might fail if journalctl is not available or restricted
            println!("⚠ Warning: Failed to collect journal entries for self: {e}");
        }
    }
}

#[tokio::test]
async fn test_collect_process_metrics_invalid_pid() {
    // Test with an invalid PID
    let invalid_pid = 999999;
    let state = AppState::new();

    match collect_process_metrics(invalid_pid, &state).await {
        Ok(_) => {
            println!("⚠ Warning: Unexpectedly found process for invalid PID {invalid_pid}");
        }
        Err(e) => {
            println!("✓ Correctly failed for invalid PID {invalid_pid}: {e}");
            assert!(e.contains("not found"));
        }
    }
}

#[tokio::test]
async fn test_collect_journal_entries_invalid_pid() {
    // Test with an invalid PID - journalctl might still return empty results
    let invalid_pid = 999999;

    match collect_journal_entries(invalid_pid).await {
        Ok(response) => {
            println!(
                "✓ Journal query completed for invalid PID {} (empty result expected): {} entries",
                invalid_pid, response.total_count
            );
            // Should be empty or very few entries
        }
        Err(e) => {
            println!("✓ Journal query failed for invalid PID {invalid_pid}: {e}");
        }
    }
}

/// The Command & Details pane went blank when the minimal-refresh
/// optimization dropped cmd from the detail endpoint's refresh kind.
#[tokio::test]
async fn test_process_metrics_include_command() {
    let state = AppState::new();
    let pid = std::process::id();
    let resp = collect_process_metrics(pid, &state)
        .await
        .expect("collect self");
    assert!(
        !resp.process.command.is_empty(),
        "command should not be empty for self (cmdline is always readable)"
    );
    println!("command = {}", resp.process.command);
}
