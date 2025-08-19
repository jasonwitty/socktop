use assert_cmd::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

fn expected_paths(config_home: &std::path::Path) -> (PathBuf, PathBuf) {
    let base = config_home.join("socktop_agent").join("tls");
    (base.join("cert.pem"), base.join("key.pem"))
}

#[test]
fn generates_self_signed_cert_and_key_in_xdg_path() {
    // Create an isolated fake XDG_CONFIG_HOME
    let tmpdir = tempfile::tempdir().expect("tempdir");
    let xdg = tmpdir.path().to_path_buf();

    // Run the agent once with --enableSSL, short timeout so it exits quickly when killed
    let mut cmd = Command::cargo_bin("socktop_agent").expect("binary exists");
    // Bind to an ephemeral port (-p 0) to avoid conflicts/flakes
    cmd.env("XDG_CONFIG_HOME", &xdg)
        .arg("--enableSSL")
        .arg("-p")
        .arg("0");

    // Spawn the process and poll for cert generation
    let mut child = cmd.spawn().expect("spawn agent");

    // Poll up to ~3s for files to appear to avoid timing flakes
    let (cert_path, key_path) = expected_paths(&xdg);
    let start = Instant::now();
    let timeout = Duration::from_millis(3000);
    let interval = Duration::from_millis(50);
    while start.elapsed() < timeout {
        if cert_path.exists() && key_path.exists() {
            break;
        }
        std::thread::sleep(interval);
    }

    // Terminate the process regardless
    let _ = child.kill();
    let _ = child.wait();

    // Verify files exist at expected paths
    assert!(
        cert_path.exists(),
        "cert not found at {}",
        cert_path.display()
    );
    assert!(key_path.exists(), "key not found at {}", key_path.display());

    // Also ensure they are non-empty
    let cert_md = fs::metadata(&cert_path).expect("cert metadata");
    let key_md = fs::metadata(&key_path).expect("key metadata");
    assert!(cert_md.len() > 0, "cert is empty");
    assert!(key_md.len() > 0, "key is empty");
}
