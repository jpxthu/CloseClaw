//! Daemon liveness assertions and graceful shutdown helpers.

use tokio::process::Child;

#[cfg(feature = "fake-llm")]
use std::time::Duration;

/// Upper bound for graceful shutdown after SIGTERM (drain timeout 30s + margin).
#[cfg(feature = "fake-llm")]
pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(40);

/// Assert that the daemon process is still alive (has not exited prematurely).
///
/// On failure, includes the exit status details and a hint to check daemon logs.
pub(crate) fn assert_daemon_alive(daemon: &mut Child) {
    assert_daemon_alive_with_context(daemon, None);
}

/// [`assert_daemon_alive`] with an optional caller-supplied context
/// phrase folded into the panic message (e.g. `Some("during gateway
/// restart")`) so failures read where in the scenario the daemon died.
pub(crate) fn assert_daemon_alive_with_context(daemon: &mut Child, context: Option<&str>) {
    if let Some(status) = daemon.try_wait().expect("try_wait daemon") {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        let context = context.map(|c| format!(" {c}")).unwrap_or_default();
        panic!(
            "daemon exited prematurely{context} with status: {:?} (exit code: {}). \
             Check daemon stdout/stderr logs for details.",
            status, code
        );
    }
}

/// Send SIGTERM to `daemon` and wait for its exit within
/// [`SHUTDOWN_TIMEOUT`]. Returns the exit status — the caller asserts
/// on it. Called only via `daemon_guard::DaemonGuard::shutdown`; its
/// callers include `agent_profile_tests` (incl. `no_bootstrap_tests`),
/// `cache_consistency_tests`, and `gateway_restart_turn_tests`
/// (terminate_daemon).
#[cfg(feature = "fake-llm")]
pub(crate) async fn sigterm_and_wait(daemon: &mut Child) -> std::process::ExitStatus {
    let pid = daemon.id().expect("daemon has a PID") as libc::pid_t;
    // SAFETY: `pid` is the PID of the daemon child this caller spawned
    // and holds; the cast to `libc::pid_t` is a lossless widening
    // conversion, and SIGTERM is a valid signal number.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    tokio::time::timeout(SHUTDOWN_TIMEOUT, daemon.wait())
        .await
        .expect("daemon should exit within the shutdown timeout")
        .expect("daemon exit status should be observable")
}
