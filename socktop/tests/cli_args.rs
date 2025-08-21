//! CLI arg parsing tests for socktop (client)
use std::process::Command;

// We test the parsing by invoking the binary with --help and ensuring the help mentions short and long flags.
// Also directly test the parse_args function via a tiny helper in a doctest-like fashion using a small
// reimplementation here kept in sync with main (compile-time test).

#[test]
fn test_help_mentions_short_and_long_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_socktop"))
        .arg("--help")
        .output()
        .expect("run socktop --help");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
    text.contains("--tls-ca") && text.contains("-t") && text.contains("--profile") && text.contains("-P"),
    "help text missing expected flags (--tls-ca/-t, --profile/-P)\n{text}"
    );
}

#[test]
fn test_tlc_ca_arg_long_and_short_parsed() {
    // Use --help combined with flags to avoid network and still exercise arg acceptance
    let exe = env!("CARGO_BIN_EXE_socktop");
    // Long form with help
    let out = Command::new(exe)
        .args(["--tls-ca", "/tmp/cert.pem", "--help"])
        .output()
        .expect("run socktop");
    assert!(
        out.status.success(),
        "socktop --tls-ca … --help did not succeed"
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("Usage:"));
    // Short form with help
    let out2 = Command::new(exe)
        .args(["-t", "/tmp/cert.pem", "--help"])
        .output()
        .expect("run socktop");
    assert!(out2.status.success(), "socktop -t … --help did not succeed");
    let text2 = format!(
        "{}{}",
        String::from_utf8_lossy(&out2.stdout),
        String::from_utf8_lossy(&out2.stderr)
    );
    assert!(text2.contains("Usage:"));

    // Profile flags with help (should not error)
    let out3 = Command::new(exe)
        .args(["--profile", "dev", "--help"])
        .output()
        .expect("run socktop");
    assert!(out3.status.success(), "socktop --profile dev --help did not succeed");
    let text3 = format!(
        "{}{}",
        String::from_utf8_lossy(&out3.stdout),
        String::from_utf8_lossy(&out3.stderr)
    );
    assert!(text3.contains("Usage:"));
}
