//! Tests for the process cache functionality

use socktop_agent::state::{AppState, CacheEntry};
use socktop_agent::types::{DetailedProcessInfo, JournalResponse, ProcessMetricsResponse};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_process_cache_ttl() {
    let state = AppState::new();
    let pid = 12345;

    // Create mock data
    let process_info = DetailedProcessInfo {
        pid,
        name: "test_process".to_string(),
        command: "test command".to_string(),
        cpu_usage: 50.0,
        mem_bytes: 1024 * 1024,
        virtual_mem_bytes: 2048 * 1024,
        shared_mem_bytes: Some(512 * 1024),
        thread_count: 4,
        fd_count: Some(10),
        status: "Running".to_string(),
        parent_pid: Some(1),
        user_id: 1000,
        group_id: 1000,
        start_time: 1234567890,
        cpu_time_user: 100000,
        cpu_time_system: 50000,
        read_bytes: Some(1024),
        write_bytes: Some(2048),
        working_directory: Some("/tmp".to_string()),
        executable_path: Some("/usr/bin/test".to_string()),
        child_processes: vec![],
        threads: vec![],
    };

    let metrics_response = ProcessMetricsResponse {
        process: process_info,
        cached_at: 1234567890,
    };

    let journal_response = JournalResponse {
        notice: None,
        entries: vec![],
        total_count: 0,
        truncated: false,
        cached_at: 1234567890,
    };

    // Test process metrics caching
    {
        let mut cache = state.cache_process_metrics.lock().await;
        cache
            .entry(pid)
            .or_insert_with(CacheEntry::new)
            .set(metrics_response.clone());
    }

    // Should get cached value immediately
    {
        let cache = state.cache_process_metrics.lock().await;
        let ttl = Duration::from_millis(250);
        if let Some(entry) = cache.get(&pid) {
            assert!(entry.is_fresh(ttl));
            assert!(entry.get().is_some());
            assert_eq!(entry.get().unwrap().process.pid, pid);
        } else {
            panic!("Expected cached entry");
        }
    }
    println!("✓ Process metrics cached and retrieved successfully");

    // Test journal entries caching
    {
        let mut cache = state.cache_journal_entries.lock().await;
        cache
            .entry(pid)
            .or_insert_with(CacheEntry::new)
            .set(journal_response.clone());
    }

    // Should get cached value immediately
    {
        let cache = state.cache_journal_entries.lock().await;
        let ttl = Duration::from_secs(1);
        if let Some(entry) = cache.get(&pid) {
            assert!(entry.is_fresh(ttl));
            assert!(entry.get().is_some());
            assert_eq!(entry.get().unwrap().total_count, 0);
        } else {
            panic!("Expected cached entry");
        }
    }
    println!("✓ Journal entries cached and retrieved successfully");

    // Wait for process metrics to expire (250ms + buffer)
    sleep(Duration::from_millis(300)).await;

    // Process metrics should be expired now
    {
        let cache = state.cache_process_metrics.lock().await;
        let ttl = Duration::from_millis(250);
        if let Some(entry) = cache.get(&pid) {
            assert!(!entry.is_fresh(ttl));
        }
    }
    println!("✓ Process metrics correctly expired after TTL");

    // Journal entries should still be valid (1s TTL)
    {
        let cache = state.cache_journal_entries.lock().await;
        let ttl = Duration::from_secs(1);
        if let Some(entry) = cache.get(&pid) {
            assert!(entry.is_fresh(ttl));
        }
    }
    println!("✓ Journal entries still valid within TTL");

    // Wait for journal entries to expire (additional 800ms to reach 1s total)
    sleep(Duration::from_millis(800)).await;

    // Journal entries should be expired now
    {
        let cache = state.cache_journal_entries.lock().await;
        let ttl = Duration::from_secs(1);
        if let Some(entry) = cache.get(&pid) {
            assert!(!entry.is_fresh(ttl));
        }
    }
    println!("✓ Journal entries correctly expired after TTL");
}
