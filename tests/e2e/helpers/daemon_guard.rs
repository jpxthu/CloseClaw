//! Shared daemon-lifecycle helpers for e2e tests.
//!
//! [`DaemonGuard`] owns the spawned daemon child and sends SIGTERM on
//! Drop, so a mid-test assertion failure never leaves a residual daemon
//! behind (`kill_on_drop` on the spawn covers the rest). The explicit
//! [`DaemonGuard::shutdown`] path goes through `sigterm_and_wait` and
//! hands the exit status back for the caller to assert.
//!
//! The spawn family builds on that guard, one layer per level of
//! boilerplate convergence (issue #3479): [`spawn_daemon_guarded`]
//! wraps a raw daemon spawn; [`spawn_ready_daemon`] additionally polls
//! both RPC sockets until ready, bounded by `READY_TIMEOUT` (the 30s
//! boot deadline previously inlined at every fake-LLM call site); and
//! [`spawn_default_ready_daemon`] converges the full default-config
//! scaffold (`start_fake_llm` → config tree → spawn → wait). The
//! caller keeps owning the `TempDir`, so guard/server lifetimes and
//! the daemon-before-tempdir drop order are unchanged.
//!
//! Extracted from `agent_profile_tests` / `cache_consistency_tests`
//! (line-for-line duplicates) so the unsafe SIGTERM block is maintained
//! in a single place (STANDARDS.md shared-helper rule).

use std::path::Path;
use std::time::Duration;

use tokio::process::Child;

use super::config::{write_config_tree, ConfigTreeOpts};
use super::daemon_lifecycle::{assert_daemon_alive, sigterm_and_wait};
use super::daemon_ready::wait_for_daemon_ready_with_timeout;
use super::fake_llm::start_fake_llm;

/// Readiness budget for [`spawn_ready_daemon`]: the 30s daemon boot
/// deadline previously inlined at every fake-LLM e2e call site.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Owns the spawned daemon child. Sending SIGTERM on Drop guarantees no
/// residual process even when an assertion fails mid-test.
///
/// The inner `Child` is exposed so existing liveness helpers
/// (`assert_daemon_alive`, `try_wait` probes) keep working on the guard;
/// shutdown still goes through [`DaemonGuard::shutdown`] or Drop.
pub(crate) struct DaemonGuard(pub(crate) Child);

impl DaemonGuard {
    /// SIGTERM + wait for graceful exit (shared `sigterm_and_wait`);
    /// returns the exit status for the caller to assert.
    pub(crate) async fn shutdown(mut self) -> std::process::ExitStatus {
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
pub(crate) fn spawn_daemon_guarded(config_root: &Path) -> DaemonGuard {
    DaemonGuard(super::daemon_spawn::spawn_daemon(config_root))
}

/// Spawn the closeclaw daemon and wait until it is ready (both RPC
/// sockets accept connections).
///
/// Converges the `spawn_daemon_guarded(config_root)` +
/// `wait_for_daemon_ready_with_timeout(config_root, 30s)` pair that was
/// duplicated at every fake-LLM e2e call site (issue #3479). Returns
/// the [`DaemonGuard`] so callers keep asserting liveness and driving
/// shutdown themselves; drop/SIGTERM semantics are unchanged.
pub(crate) async fn spawn_ready_daemon(config_root: &Path) -> DaemonGuard {
    let daemon = spawn_daemon_guarded(config_root);
    wait_for_daemon_ready_with_timeout(config_root, READY_TIMEOUT).await;
    daemon
}

/// [`spawn_ready_daemon`] over the default fake-LLM config scaffold.
///
/// Converges the full `start_fake_llm →
/// write_config_tree(ConfigTreeOpts::new(addr)) → spawn → wait`
/// boilerplate shared by the default-config e2e cases (issue #3479).
/// The caller keeps owning the `TempDir` (pass its path here), so
/// `TempDir` / [`DaemonGuard`] / fake-LLM server lifetimes and the
/// daemon-before-tempdir drop order stay exactly as before.
pub(crate) async fn spawn_default_ready_daemon(config_root: &Path) -> DaemonGuard {
    let fake_llm_addr = start_fake_llm().await;
    write_config_tree(config_root, ConfigTreeOpts::new(fake_llm_addr));
    spawn_ready_daemon(config_root).await
}

/// Standard teardown for a [`DaemonGuard`]: assert the daemon is still
/// alive, shut it down gracefully via SIGTERM, and assert it exited 0.
///
/// Converges the `assert_daemon_alive` → `shutdown` → exit-status assert
/// tail duplicated at the end of every `agent_profile_tests` case
/// (issue #3829), unifying the teardown semantics: a daemon that died
/// mid-test is now reported with exit details before the SIGTERM wait,
/// instead of surfacing as an opaque shutdown failure.
pub(crate) async fn shutdown_ok(mut daemon: DaemonGuard) {
    assert_daemon_alive(&mut daemon.0);
    let status = daemon.shutdown().await;
    assert!(status.success(), "daemon should exit 0 after SIGTERM");
}
