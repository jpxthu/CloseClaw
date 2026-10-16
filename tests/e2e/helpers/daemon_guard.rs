//! Shared daemon-lifecycle guard for e2e tests.
//!
//! [`DaemonGuard`] owns the spawned daemon child and sends SIGTERM on
//! Drop, so a mid-test assertion failure never leaves a residual daemon
//! behind (`kill_on_drop` on the spawn covers the rest). The explicit
//! [`DaemonGuard::shutdown`] path goes through `sigterm_and_wait` and
//! hands the exit status back for the caller to assert.
//!
//! Extracted from `agent_profile_tests` / `cache_consistency_tests`
//! (line-for-line duplicates) so the unsafe SIGTERM block is maintained
//! in a single place (STANDARDS.md shared-helper rule).

use std::path::Path;

use tokio::process::Child;

use super::daemon_lifecycle::sigterm_and_wait;

/// Owns the spawned daemon child. Sending SIGTERM on Drop guarantees no
/// residual process even when an assertion fails mid-test.
///
/// The inner `Child` is exposed so existing liveness helpers
/// (`assert_daemon_alive`, `try_wait` probes) keep working on the guard;
/// shutdown still goes through [`DaemonGuard::shutdown`] or Drop.
pub struct DaemonGuard(pub Child);

impl DaemonGuard {
    /// SIGTERM + wait for graceful exit (shared `sigterm_and_wait`);
    /// returns the exit status for the caller to assert.
    pub async fn shutdown(mut self) -> std::process::ExitStatus {
        sigterm_and_wait(&mut self.0).await
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        // Best-effort cleanup on panic paths: kill -TERM then drop
        // (kill_on_drop covers the rest).
        if let Some(pid) = self.0.id() {
            // SAFETY: pid belongs to this child; SIGTERM is valid.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
    }
}

/// Spawn the closeclaw daemon with the given config root, wrapped in a
/// [`DaemonGuard`] for SIGTERM-on-drop safety.
///
/// Wraps [`spawn_daemon`](super::daemon_spawn::spawn_daemon), which sets
/// `HOME` to `config_root` for PID file isolation.
pub fn spawn_daemon_guarded(config_root: &Path) -> DaemonGuard {
    DaemonGuard(super::daemon_spawn::spawn_daemon(config_root))
}
