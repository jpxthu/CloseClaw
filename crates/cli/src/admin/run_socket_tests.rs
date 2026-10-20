//! Unit tests for the admin socket probe path.
//!
//! The `wait_for_socket` cases are migrated verbatim from `run_tests.rs`
//! (issue #3185 Step 1.2); the `try_connect` cases are added by issue
//! #3831 Step 1.2 to lock in that the per-attempt timeout wrapping does
//! not change the pre-existing success / fast-failure semantics.
//!
//! Not covered — "connect hangs" (full accept backlog), issue #3831
//! evaluation note: a unix-socket connect cannot be made to pend
//! deterministically inside a UT. A bound-but-never-accepting listener
//! still completes `connect` immediately on Linux (the probe connection
//! is queued in the accept backlog), so both the old bare connect and
//! the new timeout-wrapped connect succeed in every reproducible setup —
//! the hang → per-attempt-timeout branch therefore has no direct UT, and
//! the cases below pin the observable regression surface instead.

use super::run::{try_connect, wait_for_socket};
use super::TEST_SOCKET_WAIT_TIMEOUT_MS;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::net::UnixListener;

// ═══════════════════════════════════════════════════════════════════════════
// wait_for_socket direct unit tests (issue #3185 Step 1.2)
// ═══════════════════════════════════════════════════════════════════════════
//
// Behavior dimensions:
// 1. Success: listener already bound at the socket path → Ok on the first
//    probe without consuming the timeout budget
// 2. Timeout: no listener behind the path (nonexistent path / stale socket
//    file) → Err containing "Timed out waiting for daemon admin socket"
// 3. Late success: listener becomes ready mid-wait → returns promptly on
//    the first successful probe, no extra poll-interval-grade delay

/// Success path: a listener is already bound at the socket path, so
/// wait_for_socket must return Ok on the first probe, well before the
/// timeout is exhausted.
///
/// Socket path is injected via TempDir (never the real `~/.closeclaw`).
#[tokio::test]
async fn test_wait_for_socket_success_listener_ready() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("admin.sock");
    let _listener = UnixListener::bind(&sock).expect("bind test listener");

    let started = Instant::now();
    let result = wait_for_socket(&sock, 1_000).await;
    let elapsed = started.elapsed();

    assert!(
        result.is_ok(),
        "should succeed when the listener is ready: {result:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1_000),
        "must return before exhausting the timeout budget, got {elapsed:?}"
    );
}

/// Timeout path: the socket path does not exist → wait_for_socket must
/// return Err after the tiny injected timeout, with the exact timeout
/// message including the path.
#[tokio::test]
async fn test_wait_for_socket_timeout_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("never_bound.sock");
    assert!(!sock.exists(), "precondition: socket path must not exist");

    let result = wait_for_socket(&sock, TEST_SOCKET_WAIT_TIMEOUT_MS).await;

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Timed out waiting for daemon admin socket"),
        "error should be the socket-wait timeout, got: {err_msg}"
    );
    assert!(
        err_msg.contains(sock.to_str().unwrap()),
        "error should include the socket path, got: {err_msg}"
    );
}

/// Timeout path: the socket file exists but nothing is listening behind
/// it (stale socket file) → every connect probe fails → Err after the
/// timeout, with the same timeout message.
#[tokio::test]
async fn test_wait_for_socket_timeout_stale_socket_file() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("stale.sock");
    // Bind then drop: the socket file persists on disk without a listener.
    drop(UnixListener::bind(&sock).expect("bind throwaway listener"));
    assert!(
        sock.exists(),
        "socket file should persist after the listener is dropped"
    );

    let result = wait_for_socket(&sock, TEST_SOCKET_WAIT_TIMEOUT_MS).await;

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Timed out waiting for daemon admin socket"),
        "error should be the socket-wait timeout, got: {err_msg}"
    );
}

/// Late-success boundary: the listener becomes ready mid-wait (after the
/// first probe fails). wait_for_socket must return Ok as soon as a probe
/// succeeds — total elapsed must stay below the timeout plus a small
/// margin, i.e. returning must not be delayed by extra poll intervals or
/// gated on deadline exhaustion (guards against regressing to a blocking
/// or timeout-exhausting wait).
#[tokio::test]
async fn test_wait_for_socket_late_success_within_timeout() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("late.sock");
    const TIMEOUT_MS: u64 = 500;
    const BIND_DELAY_MS: u64 = 50;

    // Bind the listener BIND_DELAY_MS into the wait, then keep accepting
    // probe connections so it stays alive until the runtime shuts the
    // task down at test end (same mock-server shape as chat rpc tests).
    let late_sock = sock.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(BIND_DELAY_MS)).await;
        let listener = UnixListener::bind(&late_sock).expect("bind late listener");
        loop {
            if listener.accept().await.is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let result = wait_for_socket(&sock, TIMEOUT_MS).await;
    let elapsed = started.elapsed();

    assert!(
        result.is_ok(),
        "should succeed once the listener becomes ready: {result:?}"
    );
    assert!(
        elapsed < Duration::from_millis(TIMEOUT_MS + 100),
        "late success must not consume the full timeout plus extra poll \
         intervals, got {elapsed:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// try_connect direct unit tests (issue #3831 Step 1.2)
// ═══════════════════════════════════════════════════════════════════════════
//
// Behavior dimensions:
// 1. Success: a bound (never-accepting) listener → Ok; the per-attempt
//    timeout wrapper must not change "connect completes without accept"
// 2. Fast failure: nonexistent socket path → Err keeping the plain
//    connect-failure context, not slowed into the timeout error
// 3. Boundary: stale socket file (listener dropped) → Err, same shape
// ("connect hangs" cannot be simulated in a UT — see the module doc note)

/// Success path: a listener is bound at the socket path but never
/// accepts. The probe connection is queued in the listen backlog, so
/// connect completes immediately — try_connect must still return Ok,
/// proving the timeout wrapper preserves the no-accept success semantics.
///
/// Socket path is injected via TempDir (never the real `~/.closeclaw`).
#[tokio::test]
async fn test_try_connect_success_listener_bound_no_accept() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("admin.sock");
    let _listener = UnixListener::bind(&sock).expect("bind test listener");

    let result = try_connect(&sock).await;

    assert!(
        result.is_ok(),
        "connect to a bound listener must succeed without accept: {result:?}"
    );
}

/// Fast-failure path: the socket path does not exist → the underlying
/// connect fails immediately (ENOENT), so try_connect must surface the
/// plain "socket connect failed" error carrying the path — not be held
/// until the per-attempt timeout rewrites it into a "timed out" error.
#[tokio::test]
async fn test_try_connect_fast_fail_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("never_bound.sock");
    assert!(!sock.exists(), "precondition: socket path must not exist");

    let started = Instant::now();
    let result = try_connect(&sock).await;
    let elapsed = started.elapsed();

    let err_msg = result
        .expect_err("connect to a nonexistent socket must fail")
        .to_string();
    assert!(
        err_msg.contains("socket connect failed"),
        "fast IO failure must keep the connect-failure context, got: {err_msg}"
    );
    assert!(
        err_msg.contains(sock.to_str().unwrap()),
        "error should include the socket path, got: {err_msg}"
    );
    assert!(
        !err_msg.contains("timed out"),
        "fast failure must not be rewritten into a timeout, got: {err_msg}"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "fast IO failure must return promptly, got {elapsed:?}"
    );
}

/// Boundary: stale socket file — the listener was bound and dropped, so
/// the file persists with nothing listening behind it → connect fails
/// fast (ECONNREFUSED) with the same connect-failure error shape, never
/// the per-attempt timeout error.
#[tokio::test]
async fn test_try_connect_err_stale_socket_file() {
    let tmp = TempDir::new().unwrap();
    let sock = tmp.path().join("stale.sock");
    // Bind then drop: the socket file persists on disk without a listener.
    drop(UnixListener::bind(&sock).expect("bind throwaway listener"));
    assert!(
        sock.exists(),
        "socket file should persist after the listener is dropped"
    );

    let result = try_connect(&sock).await;

    let err_msg = result
        .expect_err("connect to a stale socket file must fail")
        .to_string();
    assert!(
        err_msg.contains("socket connect failed"),
        "stale socket must fail with the connect-failure context, got: {err_msg}"
    );
    assert!(
        !err_msg.contains("timed out"),
        "stale-socket failure is fast, not a timeout, got: {err_msg}"
    );
}
