//! Type definitions for modal system

use std::time::Instant;

/// History data for process metrics rendering
pub struct ProcessHistoryData<'a> {
    pub cpu: &'a std::collections::VecDeque<f32>,
    /// Running sum of `cpu` maintained by the caller (avoids re-summing per frame)
    pub cpu_sum: f32,
    pub mem: &'a std::collections::VecDeque<u64>,
    pub io_read: &'a std::collections::VecDeque<u64>,
    pub io_write: &'a std::collections::VecDeque<u64>,
}

/// Process data for modal rendering
pub struct ProcessModalData<'a> {
    pub details: Option<&'a socktop_connector::ProcessMetricsResponse>,
    pub journal: Option<&'a socktop_connector::JournalResponse>,
    pub history: ProcessHistoryData<'a>,
    pub max_mem_bytes: u64,
    pub unsupported: bool,
    /// Whether the process-kill feature is available (agent local, no policy
    /// override). Only used to decide whether the `t` kill hint is shown —
    /// the kill itself is gated in `App`.
    pub kill_enabled: bool,
}

/// Parameters for rendering scatter plot
pub(super) struct ScatterPlotParams<'a> {
    pub process: &'a socktop_connector::DetailedProcessInfo,
    pub main_user_ms: f64,
    pub main_system_ms: f64,
    pub max_user: f64,
    pub max_system: f64,
}

#[derive(Debug, Clone)]
pub enum ModalType {
    ConnectionError {
        message: String,
        disconnected_at: Instant,
        retry_count: u32,
        auto_retry_countdown: Option<u64>,
    },
    ProcessDetails {
        pid: u32,
    },
    About,
    Help,
    #[allow(dead_code)]
    Confirmation {
        title: String,
        message: String,
        confirm_text: String,
        cancel_text: String,
    },
    #[allow(dead_code)]
    Info {
        title: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModalAction {
    None,    // Modal didn't handle the key, pass to main window
    Handled, // Modal handled the key, don't pass to main window
    RetryConnection,
    ExitApp,
    Confirm,
    /// Confirmation modal's second affirmative: the same action, escalated.
    /// Used by the kill prompt for SIGKILL, where `Confirm` means SIGTERM.
    ConfirmForce,
    Cancel,
    Dismiss,
    SwitchToParentProcess(u32), // Switch to viewing parent process details
    /// `t` pressed while viewing a process's details — the app decides whether
    /// the agent is local and, if so, raises the kill confirmation.
    KillSelected(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModalButton {
    Retry,
    Exit,
    Confirm,
    /// Escalated affirmative on a Confirmation modal (SIGKILL for the kill
    /// prompt). Separate button rather than a separate keybinding so the
    /// destructive option has to be selected deliberately.
    ConfirmForce,
    Cancel,
    Ok,
}
