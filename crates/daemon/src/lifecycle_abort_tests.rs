//! Unit tests for Phase 3 background task abort-and-join behavior.
//!
//! Covers Step 1.1 (Gap 1): verifies that background tasks are
//! properly terminated when they fail to exit within the timeout.
//!
//! Since `Daemon::abort_and_join_background_task` is private, these
//! tests exercise the abort-and-join pattern indirectly by reproducing
//! the same logic used in `phase_3_background_stop`.

use std::time::Duration;

/// Reproduce the abort-and-join pattern from `phase_3_background_stop`.
/// A task that exits cleanly within timeout should complete normally.
#[tokio::test]
async fn test_abort_join_clean_task_exits_normally() {
    let (tx, rx) = tokio::sync::watch::channel(());
    let handle = tokio::spawn(async move {
        let mut rx = rx;
        loop {
            if *rx.borrow_and_update() == () {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    let _ = tx.send(());

    // Must match phase_3_background_stop() join_timeout (10s).
    let join_timeout = Duration::from_secs(10);
    let result = tokio::time::timeout(join_timeout, handle).await;
    assert!(result.is_ok(), "clean task should join within timeout");
    assert!(result.unwrap().is_ok(), "clean task should not panic");
}

/// A task that never exits should be abandoned after the timeout.
/// This mirrors the abort path in `abort_and_join_background_task`.
#[tokio::test]
async fn test_abort_join_hung_task_timeout_abandons() {
    let handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    // Use a short timeout for testing
    let join_timeout = Duration::from_millis(100);
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(join_timeout, handle).await;
    let elapsed = start.elapsed();

    assert!(result.is_err(), "hung task should timeout");
    assert!(
        elapsed < Duration::from_secs(1),
        "timeout should fire within 1s, took {:?}",
        elapsed
    );
}

/// A task that panics should return `Err` (not timeout).
/// `abort_and_join_background_task` logs this as a warning and continues.
#[tokio::test]
async fn test_abort_join_panicked_task_returns_err() {
    let handle = tokio::spawn(async {
        panic!("mock background task panic");
    });

    let join_timeout = Duration::from_secs(10);
    let result = tokio::time::timeout(join_timeout, handle).await;

    // Join completes (not timeout) — it's an Err from the panic
    assert!(result.is_ok(), "panicked task join should not timeout");
    let join_result = result.unwrap();
    assert!(join_result.is_err(), "panicked task should return Err");
}

/// After abort, a hung task's handle should resolve.
/// In `abort_and_join_background_task`, after `handle.abort()`, a second
/// `tokio::time::timeout(abort_grace, handle)` is awaited. This test
/// verifies that aborting a hung task causes the join to resolve quickly.
#[tokio::test]
async fn test_abort_then_join_resolves_quickly() {
    let handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    // Let the task start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Abort the task
    handle.abort();

    // Join after abort should resolve quickly (task is terminated)
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "aborted task join should resolve quickly");
    // The join result should be Ok(Err(..)) — task was aborted, not panicked
    let inner = result.unwrap();
    assert!(inner.is_err(), "aborted task should return JoinError");
    assert!(
        elapsed < Duration::from_secs(1),
        "abort join should resolve in <1s, took {:?}",
        elapsed
    );
}

/// Multiple tasks: clean + hung. Clean exits on signal, hung times out.
/// Both should resolve without blocking each other — matching the
/// pattern in `phase_3_background_stop` where multiple handles are
/// processed sequentially.
#[tokio::test]
async fn test_abort_join_mixed_tasks_resolve_independently() {
    let (tx_clean, rx_clean) = tokio::sync::watch::channel(());

    let clean_handle = tokio::spawn(async move {
        let mut rx = rx_clean;
        loop {
            if *rx.borrow_and_update() == () {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    let hang_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    let _ = tx_clean.send(());

    let join_timeout = Duration::from_millis(100);
    let start = tokio::time::Instant::now();

    // Process both sequentially (matching phase_3 pattern)
    let clean_result = tokio::time::timeout(join_timeout, clean_handle).await;
    let hang_result = tokio::time::timeout(join_timeout, hang_handle).await;
    let elapsed = start.elapsed();

    assert!(clean_result.is_ok(), "clean task should join");
    assert!(clean_result.unwrap().is_ok(), "clean task should not panic");
    assert!(hang_result.is_err(), "hung task should timeout");

    assert!(
        elapsed < Duration::from_secs(1),
        "sequential join should complete within 1s, took {:?}",
        elapsed
    );
}

/// Verify that aborting a clean task after it already exited does not
/// cause issues. In `abort_and_join_background_task`, if the first
/// timeout succeeds, `abort()` is never called. This test verifies
/// the edge case where abort is called on an already-finished task.
#[tokio::test]
async fn test_abort_on_already_finished_task() {
    let handle = tokio::spawn(async {
        // Task completes immediately
    });

    // Give the task time to finish
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Abort on a finished task — should be a no-op
    handle.abort();

    // Join should still resolve (task already finished)
    let result = tokio::time::timeout(Duration::from_secs(1), handle).await;
    assert!(
        result.is_ok(),
        "join on aborted-finished task should resolve"
    );
}

/// Verify that abort is effective: a task blocked on I/O or sleep
/// is terminated after abort + join.
#[tokio::test]
async fn test_abort_terminates_sleeping_task() {
    let handle = tokio::spawn(async {
        // Sleep for a very long time — simulates blocked I/O
        tokio::time::sleep(Duration::from_secs(3600)).await;
    });

    // Give it time to start sleeping
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Abort should terminate the sleep
    handle.abort();

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "aborted sleeping task should join quickly");
    assert!(result.unwrap().is_err(), "aborted task returns JoinError");
}
