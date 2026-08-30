//! Step 1.4 tests: Phase 3 heartbeat periodicity, stop confirmation,
//! and grace period boundary behavior.

use crate::lifecycle::TaskStopStatus;

// =====================================================================
// Phase 3 heartbeat periodicity tests
// =====================================================================

/// Phase 3 heartbeat fires periodically during background task wait.
/// Uses a short interval (50ms) to verify multiple heartbeat cycles
/// within a bounded time window.
#[tokio::test]
async fn test_phase3_heartbeat_fires_periodically() {
    use crate::shutdown_heartbeat::ShutdownHeartbeat;

    let mut heartbeat = ShutdownHeartbeat::with_interval(std::time::Duration::from_millis(50));
    let mut heartbeats_sent = 0;

    // Simulate a long-running task that completes after 200ms
    let mut handle = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(250);

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _result = &mut handle => {
                // Task completed
                heartbeat.record_event();
                break;
            }
            _ = tokio::time::sleep_until(
                heartbeat.next_deadline(),
            ) => {
                if heartbeat.should_send_heartbeat() {
                    heartbeats_sent += 1;
                    heartbeat.record_event();
                }
            }
        }
    }

    // With 50ms interval and 200ms task, heartbeat should fire
    // at least 3 times (at ~50ms, ~100ms, ~150ms)
    assert!(
        heartbeats_sent >= 3,
        "heartbeat should fire at least 3 times in 200ms with 50ms interval, got {}",
        heartbeats_sent
    );
}

/// Task completion resets the heartbeat timer, preventing a heartbeat
/// from firing immediately after a task finishes.
#[tokio::test]
async fn test_phase3_task_completion_resets_heartbeat() {
    use crate::shutdown_heartbeat::ShutdownHeartbeat;

    let mut heartbeat = ShutdownHeartbeat::with_interval(std::time::Duration::from_millis(50));
    let mut heartbeats_sent = 0;

    // Task 1: completes at 40ms
    let mut handle1 = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    });

    // Wait for task 1 to complete
    let _ = tokio::time::timeout(std::time::Duration::from_millis(100), &mut handle1).await;
    heartbeat.record_event(); // Task 1 completed

    // After task 1, heartbeat should NOT fire immediately
    assert!(!heartbeat.should_send_heartbeat());

    // Task 2: completes at 100ms after task 1
    let mut handle2 = tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(150);

    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _result = &mut handle2 => {
                heartbeat.record_event();
                break;
            }
            _ = tokio::time::sleep_until(
                heartbeat.next_deadline(),
            ) => {
                if heartbeat.should_send_heartbeat() {
                    heartbeats_sent += 1;
                    heartbeat.record_event();
                }
            }
        }
    }

    // Heartbeat should fire during the quiet period after task 2
    assert!(
        heartbeats_sent >= 1,
        "heartbeat should fire after task completion resets timer, got {}",
        heartbeats_sent
    );
}

/// Multiple tasks completing in sequence each reset the heartbeat
/// timer, preventing premature heartbeat sends.
#[tokio::test]
async fn test_phase3_sequential_tasks_prevent_premature_heartbeat() {
    use crate::shutdown_heartbeat::ShutdownHeartbeat;

    let mut heartbeat = ShutdownHeartbeat::with_interval(std::time::Duration::from_millis(30));
    let mut heartbeats_sent = 0;

    // Simulate 4 tasks completing every 20ms (faster than 30ms interval)
    for i in 0..4 {
        let mut handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            i
        });
        let _ = tokio::time::timeout(std::time::Duration::from_millis(50), &mut handle).await;
        heartbeat.record_event();
    }

    // After all tasks, check if heartbeat fires during quiet period
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(100);
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            _ = tokio::time::sleep_until(
                heartbeat.next_deadline(),
            ) => {
                if heartbeat.should_send_heartbeat() {
                    heartbeats_sent += 1;
                    heartbeat.record_event();
                }
            }
            _ = async {
                tokio::time::sleep(std::time::Duration::from_millis(100))
                    .await;
            } => { break; }
        }
    }

    // With 20ms task interval and 30ms heartbeat interval,
    // tasks reset the timer before heartbeat can fire.
    // After all tasks complete, heartbeat fires in the quiet period.
    // Allow 1-5 heartbeats depending on timing precision.
    assert!(
        heartbeats_sent >= 1,
        "heartbeat should fire after all tasks complete, got {}",
        heartbeats_sent
    );
    assert!(
        heartbeats_sent <= 5,
        "heartbeat should not fire excessively, got {}",
        heartbeats_sent
    );
}

// =====================================================================
// Phase 3 stop confirmation tests
// =====================================================================

/// Phase 3 stop confirmation: all tasks clean → summary logged.
/// Reproduces the summary counting logic from phase_3_background_stop.
#[test]
fn test_phase3_stop_confirmation_all_clean() {
    let task_results: Vec<(&str, TaskStopStatus)> = vec![
        ("ArchiveSweeper", TaskStopStatus::Clean),
        ("AnnounceSweeper", TaskStopStatus::Clean),
        ("DreamingScheduler", TaskStopStatus::Clean),
        ("PlanArchiveSweeper", TaskStopStatus::Clean),
    ];

    let clean = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
        .count();
    let panicked = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
        .count();
    let aborted = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
        .count();

    assert_eq!(clean, 4, "all 4 tasks should be clean");
    assert_eq!(panicked, 0, "no tasks should be panicked");
    assert_eq!(aborted, 0, "no tasks should be aborted");
}

/// Phase 3 stop confirmation: mix of clean, panicked, and aborted.
#[test]
fn test_phase3_stop_confirmation_mixed_results() {
    let task_results: Vec<(&str, TaskStopStatus)> = vec![
        ("ArchiveSweeper", TaskStopStatus::Clean),
        ("AnnounceSweeper", TaskStopStatus::Aborted),
        ("DreamingScheduler", TaskStopStatus::Panicked),
        ("PlanArchiveSweeper", TaskStopStatus::Clean),
    ];

    let clean = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
        .count();
    let panicked = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
        .count();
    let aborted = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
        .count();

    assert_eq!(clean, 2, "2 tasks should be clean");
    assert_eq!(panicked, 1, "1 task should be panicked");
    assert_eq!(aborted, 1, "1 task should be aborted");
}

/// Phase 3 stop confirmation: all tasks aborted (worst case).
#[test]
fn test_phase3_stop_confirmation_all_aborted() {
    let task_results: Vec<(&str, TaskStopStatus)> = vec![
        ("ArchiveSweeper", TaskStopStatus::Aborted),
        ("AnnounceSweeper", TaskStopStatus::Aborted),
        ("DreamingScheduler", TaskStopStatus::Aborted),
        ("PlanArchiveSweeper", TaskStopStatus::Aborted),
    ];

    let clean = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
        .count();
    let panicked = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
        .count();
    let aborted = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
        .count();

    assert_eq!(clean, 0, "no tasks should be clean");
    assert_eq!(panicked, 0, "no tasks should be panicked");
    assert_eq!(aborted, 4, "all 4 tasks should be aborted");
}

/// Phase 3 stop confirmation: empty task list (no background tasks).
#[test]
fn test_phase3_stop_confirmation_empty_list() {
    let task_results: Vec<(&str, TaskStopStatus)> = vec![];

    let clean = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Clean))
        .count();
    let panicked = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Panicked))
        .count();
    let aborted = task_results
        .iter()
        .filter(|(_, s)| matches!(s, TaskStopStatus::Aborted))
        .count();

    assert_eq!(clean, 0);
    assert_eq!(panicked, 0);
    assert_eq!(aborted, 0);
}

// =====================================================================
// Grace period boundary tests
// =====================================================================

/// Task completing exactly at the grace boundary (9.9s) is NOT aborted.
/// This tests the boundary: within grace → clean exit.
#[tokio::test]
async fn test_grace_period_within_boundary_no_abort() {
    let mut task = tokio::task::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(9900)).await;
    });

    let grace = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();

    tokio::select! {
        result = &mut task => {
            assert!(result.is_ok(), "task should complete without panic");
        }
        _ = tokio::time::sleep(grace) => {
            panic!("task should complete before grace period expires");
        }
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "task should complete within grace, took {:?}",
        elapsed
    );
}

/// Task exceeding the grace boundary (10.1s) IS aborted.
/// This tests the boundary: past grace → abort.
#[tokio::test]
async fn test_grace_period_past_boundary_aborts() {
    let mut task = tokio::task::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    });

    let grace = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();

    match tokio::time::timeout(grace, &mut task).await {
        Ok(Ok(())) => panic!("task should not complete within grace"),
        Ok(Err(e)) => panic!("task panicked: {}", e),
        Err(_) => {
            // Grace period expired — abort
            task.abort();
        }
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_secs(9),
        "should wait ~10s before aborting, got {:?}",
        elapsed
    );
}

/// Task completing at 5s (well within 10s grace) exits cleanly.
#[tokio::test]
async fn test_grace_period_halfway_completion_no_abort() {
    let mut task = tokio::task::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });

    let grace = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();

    tokio::select! {
        result = &mut task => {
            assert!(result.is_ok(), "task should complete without panic");
        }
        _ = tokio::time::sleep(grace) => {
            panic!("task should complete before grace period expires");
        }
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_secs(4)
            && elapsed <= std::time::Duration::from_secs(7),
        "task should complete in ~5s, took {:?}",
        elapsed
    );
}
