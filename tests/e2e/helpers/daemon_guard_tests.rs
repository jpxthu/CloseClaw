//! Contract tests for [`super::daemon_guard::shutdown_ok`] and
//! [`super::daemon_guard::shutdown_ok_with_context`] — the standard
//! teardown triple (liveness assert → SIGTERM shutdown → exit-0 assert),
//! with the context variant additionally required to fold the caller's
//! context phrase into the liveness-assertion panic message.
//!
//! The real daemon is replaced by a minimal bash probe that installs a
//! SIGTERM trap and idles, mirroring the daemon's graceful-shutdown
//! contract (SIGTERM → clean exit) without booting the closeclaw stack;
//! the real-daemon behavior is exercised by the nine `agent_profile_tests`
//! teardowns that consume the helper.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::daemon_guard::{shutdown_ok, shutdown_ok_with_context, DaemonGuard};

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

/// Spawns a probe that exits immediately with `code`, then deterministically
/// waits out its death and asserts the exit code before handing back the
/// guard (tokio caches the status, so the teardown's try_wait observes it) —
/// shared scaffold for the "daemon already dead at teardown" cases
/// (STANDARDS.md §9 — no timing coincidence).
async fn spawn_already_dead_probe(code: u8) -> DaemonGuard {
    let mut daemon = DaemonGuard(
        Command::new("bash")
            .args(["-c", &format!("exit {code}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn bash probe"),
    );
    let status = daemon.0.wait().await.expect("probe exit status");
    assert_eq!(
        status.code(),
        Some(i32::from(code)),
        "probe must exit {code} before teardown"
    );
    daemon
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
#[should_panic(expected = "daemon should exit 0 after SIGTERM, got:")]
async fn test_shutdown_ok_panics_when_daemon_exits_nonzero_on_sigterm() {
    shutdown_ok(spawn_sigterm_probe(3).await).await;
}

/// Error path: a daemon that already died mid-test trips the liveness
/// assert (before any SIGTERM is sent), reporting the premature exit.
#[tokio::test]
#[should_panic(expected = "daemon exited prematurely")]
async fn test_shutdown_ok_panics_when_daemon_already_dead() {
    shutdown_ok(spawn_already_dead_probe(3).await).await;
}

/// Normal path (context variant): a daemon alive at teardown that exits 0
/// on SIGTERM passes the full triple without panicking; context-phrase
/// observability is pinned separately by the boundary case
/// `test_shutdown_ok_with_context_panics_with_context_in_message_when_daemon_already_dead`.
#[tokio::test]
async fn test_shutdown_ok_with_context_passes_when_daemon_exits_zero_on_sigterm() {
    shutdown_ok_with_context(
        spawn_sigterm_probe(0).await,
        Some("after restart chat turn"),
    )
    .await;
}

/// Error path (context variant): a daemon alive at teardown that exits
/// non-zero on SIGTERM still trips the exit-status assert — supplying a
/// context only decorates the liveness message, it does not weaken the
/// exit-code contract.
#[tokio::test]
#[should_panic(expected = "daemon should exit 0 after SIGTERM, got:")]
async fn test_shutdown_ok_with_context_panics_when_daemon_exits_nonzero_on_sigterm() {
    shutdown_ok_with_context(
        spawn_sigterm_probe(3).await,
        Some("after restart chat turn"),
    )
    .await;
}

/// Boundary (context variant): a daemon that already died mid-test trips
/// the liveness assert with the caller's context phrase verbatim in the
/// panic message — the observability that distinguishes this variant from
/// the context-free [`shutdown_ok`].
#[tokio::test]
#[should_panic(expected = "daemon exited prematurely context-marker-mid-test with status")]
async fn test_shutdown_ok_with_context_panics_with_context_in_message_when_daemon_already_dead() {
    shutdown_ok_with_context(
        spawn_already_dead_probe(3).await,
        Some("context-marker-mid-test"),
    )
    .await;
}
