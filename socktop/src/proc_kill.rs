//! Local process termination.
//!
//! Signals are sent by socktop itself, using this process's own OS privileges,
//! via a direct `sysinfo` call. Nothing is transmitted to the agent — the
//! agent and connector have no kill capability at all. This code path is only
//! reachable once the agent has been verified to be local (see
//! [`crate::local`]), which guarantees the PID refers to a process on this
//! machine.

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System};

/// The signals socktop can send. Deliberately limited to the two btop-style
/// primaries; no arbitrary-signal chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillSignal {
    /// SIGTERM — polite request to terminate.
    Term,
    /// SIGKILL — forceful, cannot be caught.
    Kill,
}

impl KillSignal {
    fn as_sysinfo(self) -> Signal {
        match self {
            KillSignal::Term => Signal::Term,
            KillSignal::Kill => Signal::Kill,
        }
    }

    /// Human-facing label for confirmation/result messages.
    pub fn label(self) -> &'static str {
        match self {
            KillSignal::Term => "SIGTERM",
            KillSignal::Kill => "SIGKILL",
        }
    }
}

/// Is `pid` still a live local process?
///
/// A zombie counts as gone: after a kill the entry can linger until the parent
/// reaps it, and showing a row for a process that no longer runs is exactly the
/// staleness this check exists to avoid.
pub fn process_exists(pid: u32) -> bool {
    let spid = sysinfo::Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[spid]),
        false,
        ProcessRefreshKind::nothing(),
    );
    match sys.process(spid) {
        Some(p) => p.status() != sysinfo::ProcessStatus::Zombie,
        None => false,
    }
}

/// Send `signal` to local process `pid`. Returns `Ok(())` on success, or an
/// `Err` with a human-readable reason (process gone, permission denied,
/// signal unsupported on this platform).
pub fn kill_local_process(pid: u32, signal: KillSignal) -> Result<(), String> {
    let spid = sysinfo::Pid::from_u32(pid);

    // Refresh just this one PID — we don't need a full process scan to signal it.
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[spid]),
        false,
        ProcessRefreshKind::nothing(),
    );

    let Some(proc_) = sys.process(spid) else {
        return Err(format!("Process {pid} no longer exists"));
    };

    match proc_.kill_with(signal.as_sysinfo()) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "Could not send {} to PID {pid} (permission denied?)",
            signal.label()
        )),
        None => Err(format!(
            "{} is not supported on this platform",
            signal.label()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// The path that matters: a real, live, local process must actually receive
    /// the signal. Exercises the `refresh_processes_specifics` lookup as well —
    /// if that call does not populate the process map, `sys.process()` returns
    /// None and a live PID is reported as "no longer exists".
    #[test]
    fn signals_a_real_child_process() {
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep for the test");
        let pid = child.id();

        let result = kill_local_process(pid, KillSignal::Term);

        // Reap on every path before asserting, so a failing assert cannot leak a
        // 30s sleep and cannot trip clippy's zombie_processes lint.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !exited {
            let _ = child.kill();
        }
        let _ = child.wait();

        assert!(result.is_ok(), "kill_local_process returned {result:?}");
        assert!(
            exited,
            "SIGTERM was reported sent but the child never exited"
        );
    }

    #[test]
    fn reports_a_pid_that_is_gone() {
        let mut child = Command::new("true").spawn().expect("spawn true");
        let pid = child.id();
        child.wait().expect("reap");
        // The PID is now free; signalling it must fail cleanly, not panic.
        assert!(kill_local_process(pid, KillSignal::Term).is_err());
    }
}
