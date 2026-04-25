//! E2E tests for Daemon graceful shutdown
//!
//! Covers ShutdownHandle drain state machine scenarios.

use closeclaw::daemon::shutdown::ShutdownHandle;
use std::time::Duration;

/// Test 1: drain waits until busy_count reaches zero before exiting.
#[tokio::test]
async fn test_drain_waits_until_busy_count_zero() {
    let handle = ShutdownHandle::new();

    // Increment busy count 3 times
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();

    // Spawn shutdown — should not complete while busy_count > 0
    let handle_clone = handle.clone();
    let shutdown_task = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // Give it a moment to enter ShuttingDown state
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        handle.state(),
        closeclaw::daemon::shutdown::ShutdownState::ShuttingDown
    );
    assert_eq!(handle.busy_count(), 3);

    // Decrement one at a time and verify state doesn't change yet
    handle.decrement_busy();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!handle.is_stopped());

    handle.decrement_busy();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!handle.is_stopped());

    // Last decrement — drain should complete
    handle.decrement_busy();

    // Wait for shutdown to finish (3s drain timeout in test mode + buffer)
    let _ = tokio::time::timeout(Duration::from_secs(5), shutdown_task).await;

    assert!(handle.is_stopped());
}

/// Test 2: drain timeout forces exit even when busy_count > 0.
#[tokio::test]
async fn test_drain_timeout_forces_exit() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();

    let handle_clone = handle.clone();

    // Spawn initiate_shutdown and await it directly (blocks this task,
    // but the spawned task runs concurrently on the same runtime).
    // The runtime drives the spawned task's sleep to completion.
    let _ = tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    })
    .await;

    // initiate_shutdown should have completed after the 3s drain timeout
    assert!(
        handle.is_stopped(),
        "handle should be stopped (state={:?})",
        handle.state()
    );
    assert_eq!(handle.busy_count(), 1);
}

/// Test 3: drain signal is broadcast to all subscribers.
#[tokio::test]
async fn test_drain_signal_broadcast() {
    let handle = ShutdownHandle::new();

    let mut rx1 = handle.subscribe_drain();
    let mut rx2 = handle.subscribe_drain();

    // Spawn shutdown in background
    let handle_clone = handle.clone();
    tokio::spawn(async move {
        handle_clone.initiate_shutdown().await;
    });

    // Both receivers should receive the signal within 1 second
    let result1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv()).await;
    let result2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv()).await;

    assert!(result1.is_ok(), "Receiver 1 did not get drain signal");
    assert!(result2.is_ok(), "Receiver 2 did not get drain signal");
}

/// Test 4: Daemon::run() triggers graceful shutdown when receiving SIGTERM.
#[tokio::test]
async fn test_daemon_run_sigterm_shutdown() {
    // Create temp dir with minimal agents.json
    let temp_dir = tempfile::TempDir::new().expect("temp dir");
    let agents_path = temp_dir.path().join("agents.json");
    std::fs::write(&agents_path, r#"{"version":"1.0.0","agents":[]}"#).expect("write agents.json");

    // Do NOT set FEISHU/LLM env vars — Daemon::start will skip those components
    let daemon = closeclaw::daemon::Daemon::start(temp_dir.path().to_str().unwrap())
        .await
        .expect("daemon start");

    let pid = std::process::id();

    // Spawn a task that sends SIGTERM to this process after a short delay.
    // This mirrors what an external signal source would do.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        // SAFETY: pid is our own process, this is safe for sending SIGTERM.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    });

    // Call Daemon::run() — it blocks on signal reception. When SIGTERM is sent
    // (from the spawned task above), run() initiates shutdown and returns.
    let _ = daemon.run().await;

    // Verify stopped within 5 seconds: poll is_stopped() until it returns true
    let poll_result: Result<(), tokio::time::error::Elapsed> =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if daemon.shutdown.is_stopped() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
    assert!(
        poll_result.is_ok(),
        "daemon should be stopped within 5s (state={:?})",
        daemon.shutdown.state()
    );
}
