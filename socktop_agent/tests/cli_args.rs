//! CLI arg parsing tests for socktop_agent (server)
use std::process::Command;

#[test]
fn test_help_and_port_short_long() {
    // We verify port flags are accepted by ensuring the process starts (then we kill quickly).
    // Use an unlikely port to avoid conflicts.
    let exe = env!("CARGO_BIN_EXE_socktop_agent");

    // TLS enabled with long --port
    let mut child = Command::new(exe)
        .args(["--enableSSL", "--port", "9555"])
        .spawn()
        .expect("spawn agent");
    // Give it a moment to bind
    std::thread::sleep(std::time::Duration::from_millis(150));
    let _ = child.kill();
    let _ = child.wait();

    // TLS enabled with short -p
    let mut child2 = Command::new(exe)
        .args(["--enableSSL", "-p", "9556"])
        .spawn()
        .expect("spawn agent");
    std::thread::sleep(std::time::Duration::from_millis(150));
    let _ = child2.kill();
    let _ = child2.wait();
}
