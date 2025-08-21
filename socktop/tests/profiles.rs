//! Tests for profile load/save and resolution logic (non-interactive paths only)
use std::fs;
use std::sync::Mutex;

// Global lock to serialize tests that mutate process-wide environment variables.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[allow(dead_code)] // touch crate
fn touch() {
    let _ = socktop::types::Metrics {
        cpu_total: 0.0,
        cpu_per_core: vec![],
        mem_total: 0,
        mem_used: 0,
        swap_total: 0,
        swap_used: 0,
        process_count: None,
        hostname: String::new(),
        cpu_temp_c: None,
        disks: vec![],
        networks: vec![],
        top_processes: vec![],
        gpus: None,
    };
}

// We re-import internal modules by copying minimal logic here because profiles.rs isn't public.
// Instead of exposing internals, we simulate profile saving through CLI invocations.

use std::process::Command;

fn run_socktop(args: &[&str]) -> (bool, String) {
    let exe = env!("CARGO_BIN_EXE_socktop");
    let output = Command::new(exe).args(args).output().expect("run socktop");
    let ok = output.status.success();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (ok, text)
}

fn config_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(xdg).join("socktop")
    } else {
        dirs_next::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("socktop")
    }
}

fn profiles_path() -> std::path::PathBuf {
    config_dir().join("profiles.json")
}

#[test]
fn test_profile_created_on_first_use() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Isolate config in a temp dir
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", td.path());
    // Ensure directory exists fresh
    std::fs::create_dir_all(td.path().join("socktop")).unwrap();
    let _ = fs::remove_file(profiles_path());
    // Provide profile + url => should create profiles.json
    let (_ok, _out) = run_socktop(&["--profile", "unittest", "ws://example:1/ws", "--dry-run"]);
    // We pass --help to exit early after parsing (no network attempt)
    let data = fs::read_to_string(profiles_path()).expect("profiles.json created");
    assert!(
        data.contains("unittest"),
        "profiles.json missing profile entry: {data}"
    );
}

#[test]
fn test_profile_overwrite_only_when_changed() {
    let _guard = ENV_LOCK.lock().unwrap();
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", td.path());
    std::fs::create_dir_all(td.path().join("socktop")).unwrap();
    let _ = fs::remove_file(profiles_path());
    // Initial create
    let (_ok, _out) = run_socktop(&["--profile", "prod", "ws://one/ws", "--dry-run"]); // create
    let first = fs::read_to_string(profiles_path()).unwrap();
    // Re-run identical (should not duplicate or corrupt)
    let (_ok2, _out2) = run_socktop(&["--profile", "prod", "ws://one/ws", "--dry-run"]); // identical
    let second = fs::read_to_string(profiles_path()).unwrap();
    assert_eq!(
        first, second,
        "Profile file changed despite identical input"
    );
    // Overwrite with different URL using --save (no prompt path)
    let (_ok3, _out3) = run_socktop(&["--profile", "prod", "--save", "ws://two/ws", "--dry-run"]);
    let third = fs::read_to_string(profiles_path()).unwrap();
    assert!(third.contains("two"), "Updated URL not written: {third}");
}

#[test]
fn test_profile_tls_ca_persisted() {
    let _guard = ENV_LOCK.lock().unwrap();
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", td.path());
    std::fs::create_dir_all(td.path().join("socktop")).unwrap();
    let _ = fs::remove_file(profiles_path());
    let (_ok, _out) = run_socktop(&[
        "--profile",
        "secureX",
        "--tls-ca",
        "/tmp/cert.pem",
        "wss://host/ws",
        "--dry-run",
    ]);
    let data = fs::read_to_string(profiles_path()).unwrap();
    assert!(data.contains("secureX"));
    assert!(data.contains("cert.pem"));
}
