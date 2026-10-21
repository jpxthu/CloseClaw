//! Contract tests for [`super::daemon_guard::shutdown_ok`] — the standard
//! teardown triple (liveness assert → SIGTERM shutdown → exit-0 assert).
//!
//! The real daemon is replaced by a minimal bash probe that installs a
//! SIGTERM trap and idles, mirroring the daemon's graceful-shutdown
//! contract (SIGTERM → clean exit) without booting the closeclaw stack;
//! the real-daemon behavior is exercised by the nine `agent_profile_tests`
//! teardowns that consume the helper.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::daemon_guard::{shutdown_ok, DaemonGuard};

/// Spawns a probe process whose trap is proven armed when this returns:
/// `bash` installs a SIGTERM trap exiting with `code`, then writes a
/// ready line to piped stdout; the parent proceeds only after reading it,
/// closing the exec→trap registration race window (STANDARDS.md §9 — no
/// timing coincidence). Afterwards the probe idles in 100ms sleeps, so
/// the trap fires within ~100ms — far inside the 40s shutdown timeout.
/// `kill_on_drop` mirrors [`super::daemon_spawn::spawn_daemon`].
async fn spawn_sigterm_probe(code: u8) -> DaemonGuard {
    let snippet = format!("trap 'exit {code}' TERM; echo ready; while :; do sleep 0.1; done");
    let mut child = Command::new("bash")
        .args(["-c", &snippet])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn bash probe");
    let mut stdout = BufReader::new(child.stdout.take().expect("probe stdout pipe"));
    let mut ready = Vec::new();
    stdout
        .read_until(b'\n', &mut ready)
        .await
        .expect("read probe ready line");
    DaemonGuard(child)
}

/// Normal path: a daemon alive at teardown that exits 0 on SIGTERM passes
/// the full triple without panicking.
#[tokio::test]
async fn test_shutdown_ok_passes_when_daemon_exits_zero_on_sigterm() {
    shutdown_ok(spawn_sigterm_probe(0).await).await;
}

/// Error path: a daemon alive at teardown that exits non-zero on SIGTERM
/// trips the exit-status assert.
#[tokio::test]
#[should_panic(expected = "daemon should exit 0 after SIGTERM")]
async fn test_shutdown_ok_panics_when_daemon_exits_nonzero_on_sigterm() {
    shutdown_ok(spawn_sigterm_probe(3).await).await;
}

/// Error path: a daemon that already died mid-test trips the liveness
/// assert (before any SIGTERM is sent), reporting the premature exit.
#[tokio::test]
#[should_panic(expected = "daemon exited prematurely")]
async fn test_shutdown_ok_panics_when_daemon_already_dead() {
    let child = Command::new("bash")
        .args(["-c", "exit 3"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn bash probe");
    let mut daemon = DaemonGuard(child);
    // Deterministically wait out the probe death (tokio caches the status,
    // so the teardown's try_wait observes it), and assert the probe really
    // died with the non-zero code this scenario requires.
    let status = daemon.0.wait().await.expect("probe exit status");
    assert_eq!(status.code(), Some(3), "probe must exit 3 before teardown");

    shutdown_ok(daemon).await;
}
