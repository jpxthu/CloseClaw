//! Unit tests for BackgroundTaskManager.
//!
//! These tests are compiled via `#[path = "background_tests.rs"]` inside
//! `background.rs`'s `#[cfg(test)] mod tests`.

use super::*;
use std::time::Duration;
use tempfile::TempDir;

/// Helper: create a manager with a temp dir.
fn test_manager() -> (BackgroundTaskManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let mgr = BackgroundTaskManager::with_temp_dir(tmp.path());
    (mgr, tmp)
}

/// Helper: wait for a task to leave the Running state, returning the snapshot.
async fn wait_for_completion(mgr: &BackgroundTaskManager, task_id: &str) -> BackgroundTask {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = mgr.get_task(task_id).await.unwrap();
            if !matches!(snapshot.state, TaskState::Running { .. }) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("task did not complete within timeout")
}

// ---------------------------------------------------------------------------
// 1. spawn basic — returns Running immediately
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_returns_running() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("echo hello", _tmp.path(), false).await.unwrap();
    assert!(
        matches!(
            task.state,
            TaskState::Running {
                is_backgrounded: false
            }
        ),
        "explicit spawn should not be backgrounded"
    );
}

// ---------------------------------------------------------------------------
// 2. task completes successfully
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_completes() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    assert_eq!(snapshot.state, TaskState::Completed { exit_code: 0 });
}

// ---------------------------------------------------------------------------
// 3. task fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_fails() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("false", _tmp.path(), false).await.unwrap();
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    assert_eq!(snapshot.state, TaskState::Failed { exit_code: 1 });
}

#[tokio::test]
async fn test_spawn_nonexistent_command() {
    let (mgr, _tmp) = test_manager();
    let task = mgr
        .spawn("nonexistent_cmd_xyz_12345", _tmp.path(), false)
        .await
        .unwrap();
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    match snapshot.state {
        TaskState::Failed { exit_code } => assert!(exit_code != 0),
        other => panic!("expected Failed state, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 4. kill
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    // Give the spawned process time to set its handle
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(mgr.is_running(&task.id).await);
    mgr.kill(&task.id).await.unwrap();
    let snapshot = mgr.get_task(&task.id).await.unwrap();
    assert_eq!(snapshot.state, TaskState::Killed);
}

#[tokio::test]
async fn test_kill_non_running_returns_error() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    let result = mgr.kill(&task.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_kill_nonexistent_task() {
    let (mgr, _tmp) = test_manager();
    let result = mgr.kill("nonexistent-id").await;
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// 5. is_running
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_is_running() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    assert!(mgr.is_running(&task.id).await);
    let _ = wait_for_completion(&mgr, &task.id).await;
    assert!(!mgr.is_running(&task.id).await);
}

#[tokio::test]
async fn test_is_running_nonexistent() {
    let (mgr, _tmp) = test_manager();
    assert!(!mgr.is_running("nonexistent-id").await);
}

// ---------------------------------------------------------------------------
// 6. get_task — snapshot
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_task() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("echo hello", _tmp.path(), false).await.unwrap();
    let snapshot = mgr.get_task(&task.id).await;
    assert!(snapshot.is_some());
    let s = snapshot.unwrap();
    assert_eq!(s.id, task.id);
    assert_eq!(s.command, "echo hello");
    assert!(
        matches!(
            s.state,
            TaskState::Running {
                is_backgrounded: false
            }
        ),
        "explicit spawn snapshot should not be backgrounded"
    );
}

#[tokio::test]
async fn test_get_task_nonexistent() {
    let (mgr, _tmp) = test_manager();
    assert!(mgr.get_task("nonexistent-id").await.is_none());
}

// ---------------------------------------------------------------------------
// 7. output file — stdout and stderr captured
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_output_file_captures_stdout() {
    let (mgr, _tmp) = test_manager();
    let task = mgr
        .spawn("echo hello_output", _tmp.path(), false)
        .await
        .unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    let content = tokio::fs::read_to_string(&task.output_path).await.unwrap();
    assert!(content.contains("hello_output"));
}

#[tokio::test]
async fn test_output_file_captures_stderr() {
    let (mgr, _tmp) = test_manager();
    let task = mgr
        .spawn("echo hello_stderr >&2", _tmp.path(), false)
        .await
        .unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    let content = tokio::fs::read_to_string(&task.output_path).await.unwrap();
    assert!(content.contains("hello_stderr"));
}

// ---------------------------------------------------------------------------
// 8. pending_notifications — completed task returns Later notification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pending_notifications_on_complete() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let notifs = mgr.pending_notifications().await;
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].task_id, task.id);
    assert_eq!(notifs[0].priority, NotificationPriority::Later);
}

#[tokio::test]
async fn test_pending_notifications_on_failure() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("false", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let notifs = mgr.pending_notifications().await;
    assert_eq!(notifs.len(), 1);
    match &notifs[0].state {
        TaskState::Failed { exit_code } => assert_eq!(*exit_code, 1),
        other => panic!("expected Failed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 9. notification dedup — multiple drains return empty after first
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notification_dedup() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let first = mgr.pending_notifications().await;
    assert!(!first.is_empty(), "first drain should return notifications");

    let second = mgr.pending_notifications().await;
    assert!(second.is_empty(), "second drain should be empty (dedup)");
}

// ---------------------------------------------------------------------------
// 10. mark_notified — flag set, completion notification still in queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_mark_notified() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Mark the task as notified (prevents stuck detection from adding duplicates)
    mgr.mark_notified(&task.id).await;

    // The completion notification is still in the queue (mark_notified
    // only affects stuck detection dedup, not finalize_state's push)
    let notifs = mgr.pending_notifications().await;
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].task_id, task.id);
}

// ---------------------------------------------------------------------------
// 11. backgroundize — no cwd parameter (Step 1.2 / Step 1.3)
//
// Refs: issue #814 Step 1.2 — `backgroundize(child, command)` signature,
// dropping the unused `_cwd` parameter. The fact that this test compiles
// verifies the new signature; the runtime assertions verify the
// take-over behavior is unchanged.
// ---------------------------------------------------------------------------

/// Helper: spawn a `sh -c <command>` child and return the Child + its
/// stdout/stderr handles. Mirrors what BashTool does in the foreground
/// path before handing the child to the manager.
async fn spawn_test_child(
    command: &str,
) -> (
    tokio::process::Child,
    Option<tokio::process::ChildStdout>,
    Option<tokio::process::ChildStderr>,
) {
    use tokio::process::Command;
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn test child");
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    (child, stdout, stderr)
}

#[tokio::test]
async fn test_backgroundize_signature_no_cwd() {
    // This test exists to exercise the new `backgroundize(child, command)`
    // signature: NO `cwd` argument. If the signature regresses, this
    // file will fail to compile.
    let (mgr, _tmp) = test_manager();
    let (child, _stdout, _stderr) = spawn_test_child("true").await;

    let task = mgr
        .backgroundize(child, "true", true)
        .await
        .expect("backgroundize(child, command) should succeed");

    assert!(
        matches!(
            task.state,
            TaskState::Running {
                is_backgrounded: true
            }
        ),
        "auto-backgroundize task should be backgrounded"
    );
    assert_eq!(task.command, "true");
    assert!(mgr.is_running(&task.id).await);

    // Eventually completes successfully.
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    assert_eq!(snapshot.state, TaskState::Completed { exit_code: 0 });
}

#[tokio::test]
async fn test_backgroundize_takes_over_long_running_child() {
    // Verify the take-over path: hand off a long-running child, then
    // kill it via the manager. The child is owned by the manager after
    // `backgroundize`, so a subsequent `kill` must succeed.
    let (mgr, _tmp) = test_manager();
    let (child, _stdout, _stderr) = spawn_test_child("sleep 60").await;

    let task = mgr
        .backgroundize(child, "sleep 60", true)
        .await
        .expect("backgroundize should accept a long-running child");
    assert!(
        matches!(
            task.state,
            TaskState::Running {
                is_backgrounded: true
            }
        ),
        "auto-backgroundize long-running task should be backgrounded"
    );
    assert!(mgr.is_running(&task.id).await);

    // Give the manager a moment to wire up the kill_tx.
    tokio::time::sleep(Duration::from_millis(200)).await;

    mgr.kill(&task.id).await.expect("kill should succeed");
    let snapshot = mgr.get_task(&task.id).await.unwrap();
    assert_eq!(snapshot.state, TaskState::Killed);
}

#[tokio::test]
async fn test_backgroundize_captures_child_output() {
    // Verify that stdout/stderr from the taken-over child are captured
    // into the task output file. The auto-background path in BashTool
    // relies on this for the LLM to later read the command's output.
    let (mgr, _tmp) = test_manager();
    let (mut child, stdout_handle, stderr_handle) = spawn_test_child("echo bgize_output").await;

    // `spawn_test_child` extracted stdout/stderr so the caller can decide
    // whether to consume them. `backgroundize()` reads them itself via
    // `child.stdout.take()`, so we must put the handles back on the child
    // first — mirroring what `BashTool::handle_foreground_result` does in
    // the auto-background path.
    child.stdout = stdout_handle;
    child.stderr = stderr_handle;

    let task = mgr
        .backgroundize(child, "echo bgize_output", true)
        .await
        .expect("backgroundize should succeed");
    assert!(
        matches!(
            task.state,
            TaskState::Running {
                is_backgrounded: true
            }
        ),
        "auto-backgroundize output capture task should be backgrounded"
    );
    let _ = wait_for_completion(&mgr, &task.id).await;

    let content = tokio::fs::read_to_string(&task.output_path)
        .await
        .expect("output file should be readable");
    assert!(
        content.contains("bgize_output"),
        "expected captured stdout in {:?}, got: {:?}",
        task.output_path,
        content
    );
}

// =========================================================================
// TaskState — type-level tests
// =========================================================================

#[test]
fn test_task_state_running() {
    let state = TaskState::Running {
        is_backgrounded: false,
    };
    assert_eq!(
        state,
        TaskState::Running {
            is_backgrounded: false
        }
    );
}

#[test]
fn test_task_state_completed() {
    let state = TaskState::Completed { exit_code: 0 };
    match state {
        TaskState::Completed { exit_code } => assert_eq!(exit_code, 0),
        _ => panic!("expected Completed"),
    }
}

#[test]
fn test_task_state_failed() {
    let state = TaskState::Failed { exit_code: 1 };
    match state {
        TaskState::Failed { exit_code } => assert_eq!(exit_code, 1),
        _ => panic!("expected Failed"),
    }
}

#[test]
fn test_task_state_killed() {
    let state = TaskState::Killed;
    assert_eq!(state, TaskState::Killed);
}

#[test]
fn test_task_state_clone() {
    let original = TaskState::Completed { exit_code: 42 };
    let cloned = original.clone();
    assert_eq!(original, cloned);
}

#[test]
fn test_task_state_debug() {
    let states = [
        TaskState::Running {
            is_backgrounded: false,
        },
        TaskState::Completed { exit_code: 0 },
        TaskState::Failed { exit_code: 1 },
        TaskState::Killed,
    ];
    for s in &states {
        let debug = format!("{:?}", s);
        assert!(!debug.is_empty());
    }
}

#[test]
fn test_task_state_equality_distinct_variants() {
    assert_ne!(
        TaskState::Running {
            is_backgrounded: false
        },
        TaskState::Completed { exit_code: 0 }
    );
    assert_ne!(
        TaskState::Running {
            is_backgrounded: false
        },
        TaskState::Failed { exit_code: 1 }
    );
    assert_ne!(
        TaskState::Running {
            is_backgrounded: false
        },
        TaskState::Killed
    );
    assert_ne!(
        TaskState::Completed { exit_code: 0 },
        TaskState::Failed { exit_code: 0 }
    );
}

// =========================================================================
// BackgroundTask — construction and derived traits
// =========================================================================

#[test]
fn test_background_task_fields() {
    let task = BackgroundTask {
        id: "abc-123".to_string(),
        command: "echo hello".to_string(),
        state: TaskState::Running {
            is_backgrounded: false,
        },
        output_path: PathBuf::from("/tmp/out"),
    };
    assert_eq!(task.id, "abc-123");
    assert_eq!(task.command, "echo hello");
    assert!(matches!(task.state, TaskState::Running { .. }));
    assert_eq!(task.output_path, PathBuf::from("/tmp/out"));
}

#[test]
fn test_background_task_clone() {
    let task = BackgroundTask {
        id: "clone-id".to_string(),
        command: "ls".to_string(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: PathBuf::from("/tmp/clone"),
    };
    let cloned = task.clone();
    assert_eq!(cloned.id, task.id);
    assert_eq!(cloned.command, task.command);
    assert_eq!(cloned.state, task.state);
    assert_eq!(cloned.output_path, task.output_path);
}

#[test]
fn test_background_task_debug() {
    let task = BackgroundTask {
        id: "debug-id".to_string(),
        command: "pwd".to_string(),
        state: TaskState::Running {
            is_backgrounded: false,
        },
        output_path: PathBuf::from("/tmp/debug"),
    };
    let debug = format!("{:?}", task);
    assert!(debug.contains("BackgroundTask"));
    assert!(debug.contains("debug-id"));
}

// =========================================================================
// BackgroundTaskError — Display and variant tests
// =========================================================================

#[test]
fn test_background_task_error_spawn_failed_display() {
    let err = BackgroundTaskError::SpawnFailed("permission denied".into());
    assert_eq!(format!("{}", err), "spawn failed: permission denied");
}

#[test]
fn test_background_task_error_not_found_display() {
    let err = BackgroundTaskError::NotFound("task-42".into());
    assert_eq!(format!("{}", err), "task not found: task-42");
}

#[test]
fn test_background_task_error_not_running_display() {
    let err = BackgroundTaskError::NotRunning("task-99".into());
    assert_eq!(format!("{}", err), "task not running: task-99");
}

#[test]
fn test_background_task_error_io_display() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let err = BackgroundTaskError::Io(io_err);
    let msg = format!("{}", err);
    assert!(msg.contains("io error"));
    assert!(msg.contains("file missing"));
}

#[test]
fn test_background_task_error_debug() {
    let err = BackgroundTaskError::SpawnFailed("test".into());
    let debug = format!("{:?}", err);
    assert!(debug.contains("SpawnFailed"));
}

// =========================================================================
// cleanup_finished — terminal task removal
// =========================================================================

/// Helper: insert a task handle directly into the manager's internal map.
/// Returns the output_path so tests can check existence.
async fn insert_handle(
    mgr: &BackgroundTaskManager,
    task_id: &str,
    command: &str,
    state: TaskState,
) -> PathBuf {
    let tmp = mgr.temp_dir.join("closeclaw/background").join(task_id);
    let output_path = tmp.join("output");
    // Ensure the parent dir exists so tokio::fs::remove_dir_all has
    // something to remove.
    tokio::fs::create_dir_all(&tmp).await.unwrap();
    tokio::fs::write(&output_path, "test output").await.unwrap();
    let handle = TaskHandle {
        id: task_id.to_owned(),
        command: command.to_owned(),
        state,
        output_path: output_path.clone(),
        kill_tx: None,
        notified: false,
    };
    mgr.tasks.lock().await.insert(task_id.to_owned(), handle);
    output_path
}

/// Verify cleanup_finished removes output directories and handles for
/// Completed and Failed tasks, but preserves output for Killed tasks.
#[tokio::test]
async fn test_cleanup_finished_removes_terminal_tasks() {
    let (mgr, _tmp) = test_manager();
    let running_path = insert_handle(
        &mgr,
        "t-run",
        "echo hi",
        TaskState::Running {
            is_backgrounded: false,
        },
    )
    .await;
    let completed_path = insert_handle(
        &mgr,
        "t-completed",
        "true",
        TaskState::Completed { exit_code: 0 },
    )
    .await;
    let failed_path = insert_handle(
        &mgr,
        "t-failed",
        "false",
        TaskState::Failed { exit_code: 1 },
    )
    .await;
    let killed_path = insert_handle(&mgr, "t-killed", "sleep 99", TaskState::Killed).await;

    mgr.cleanup_finished().await;

    // Completed/Failed: output dir and handle should be gone.
    assert!(!completed_path.exists());
    assert!(mgr.get_task("t-completed").await.is_none());
    assert!(!failed_path.exists());
    assert!(mgr.get_task("t-failed").await.is_none());

    // Killed: handle removed but output directory preserved.
    assert!(mgr.get_task("t-killed").await.is_none());
    assert!(killed_path.exists());

    // Running task: output file and handle still present.
    assert!(running_path.exists());
    assert!(mgr.get_task("t-run").await.is_some());
}

// ---------------------------------------------------------------------------
// Gap 1 tests — cleanup_finished must preserve Killed output
// ---------------------------------------------------------------------------

/// Cleanup of a Killed task removes the handle from the map.
#[tokio::test]
async fn test_cleanup_finished_killed_handle_removed() {
    let (mgr, _tmp) = test_manager();
    insert_handle(&mgr, "k1", "sleep 1", TaskState::Killed).await;

    mgr.cleanup_finished().await;

    assert!(
        mgr.get_task("k1").await.is_none(),
        "Killed task handle must be evicted from the map"
    );
}

/// Cleanup of a Killed task must NOT delete the output directory.
#[tokio::test]
async fn test_cleanup_finished_killed_output_preserved() {
    let (mgr, _tmp) = test_manager();
    let output_path = insert_handle(&mgr, "k2", "sleep 2", TaskState::Killed).await;

    mgr.cleanup_finished().await;

    assert!(
        output_path.exists(),
        "Killed task output file must survive cleanup"
    );
    let parent = output_path.parent().unwrap();
    assert!(
        parent.exists(),
        "Killed task output directory must survive cleanup"
    );
}

/// Cleanup of a Completed task deletes both handle and output directory.
#[tokio::test]
async fn test_cleanup_finished_completed_removes_output() {
    let (mgr, _tmp) = test_manager();
    let output_path =
        insert_handle(&mgr, "c1", "true", TaskState::Completed { exit_code: 0 }).await;

    mgr.cleanup_finished().await;

    assert!(!output_path.exists());
    assert!(mgr.get_task("c1").await.is_none());
}

/// Cleanup of a Failed task deletes both handle and output directory.
#[tokio::test]
async fn test_cleanup_finished_failed_removes_output() {
    let (mgr, _tmp) = test_manager();
    let output_path = insert_handle(&mgr, "f1", "false", TaskState::Failed { exit_code: 1 }).await;

    mgr.cleanup_finished().await;

    assert!(!output_path.exists());
    assert!(mgr.get_task("f1").await.is_none());
}

/// Verify that Running tasks are not touched by cleanup_finished.
#[tokio::test]
async fn test_cleanup_finished_preserves_running_tasks() {
    let (mgr, _tmp) = test_manager();
    let running_path = insert_handle(
        &mgr,
        "run-1",
        "echo hello",
        TaskState::Running {
            is_backgrounded: false,
        },
    )
    .await;

    mgr.cleanup_finished().await;

    assert!(running_path.exists(), "Running task output should survive");
    assert!(
        mgr.get_task("run-1").await.is_some(),
        "Running task handle should survive"
    );
}

/// Calling cleanup_finished twice must not panic or error.
#[tokio::test]
async fn test_cleanup_finished_idempotent() {
    let (mgr, _tmp) = test_manager();
    let completed_path = insert_handle(
        &mgr,
        "idem-1",
        "true",
        TaskState::Completed { exit_code: 0 },
    )
    .await;

    mgr.cleanup_finished().await;
    assert!(!completed_path.exists());

    // Second call on an already-cleaned manager.
    mgr.cleanup_finished().await;
    assert!(!completed_path.exists());
}

/// When the output directory does not exist (already deleted externally),
/// cleanup_finished should only warn — never panic.
#[tokio::test]
async fn test_cleanup_finished_cleanup_io_error() {
    let (mgr, _tmp) = test_manager();
    let output = _tmp.path().join("no-such-dir").join("output");
    let handle = TaskHandle {
        id: "io-err".to_owned(),
        command: "test".to_owned(),
        state: TaskState::Completed { exit_code: 0 },
        output_path: output,
        kill_tx: None,
        notified: false,
    };
    mgr.tasks.lock().await.insert("io-err".to_owned(), handle);

    // Should not panic — remove_dir_all on a missing path logs a warning
    // and moves on.
    mgr.cleanup_finished().await;
    assert!(mgr.get_task("io-err").await.is_none());
}

// =========================================================================
// Step 1.5 — Summary text verification (Step 1.1 验证)
// =========================================================================

/// Verify that a completion notification carries the correct summary
/// text: "Background command '<command>' completed".
#[tokio::test]
async fn test_completion_notification_summary_text() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("echo hello", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let notifs = mgr.pending_notifications().await;
    assert_eq!(notifs.len(), 1);
    assert_eq!(
        notifs[0].summary, "Background command 'echo hello' completed",
        "summary should match the expected format"
    );
}

/// Verify that a failure notification carries the correct summary text.
#[tokio::test]
async fn test_failure_notification_summary_text() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("false", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let notifs = mgr.pending_notifications().await;
    assert_eq!(notifs.len(), 1);
    assert_eq!(
        notifs[0].summary, "Background command 'false' completed",
        "failure summary should match the expected format"
    );
}

// =========================================================================
// Step 1.5 — Dedup via notified flag (Step 1.2 验证)
// =========================================================================

/// When `notified` is already `true` before `finalize_state` runs,
/// no completion notification should be pushed. This simulates the
/// stuck-then-completion path where `emit_stuck_alert` set the flag.
#[tokio::test]
async fn test_finalize_state_skips_notification_when_notified() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Drain the first (legitimate) notification.
    let first = mgr.pending_notifications().await;
    assert_eq!(first.len(), 1, "first drain should have one notification");

    // Mark as notified (simulates stuck alert having fired).
    mgr.mark_notified(&task.id).await;

    // Second drain should be empty — no duplicate notification.
    let second = mgr.pending_notifications().await;
    assert!(
        second.is_empty(),
        "second drain should be empty when notified is true"
    );
}

// =========================================================================
// Step 1.5 — Killed state: finalize_state early return (Step 1.2 验证)
// =========================================================================

/// A killed task should not produce any completion notification.
/// `finalize_state` returns early when state is `Killed`.
#[tokio::test]
async fn test_killed_task_produces_no_notification() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(mgr.is_running(&task.id).await);

    mgr.kill(&task.id).await.unwrap();
    let snapshot = mgr.get_task(&task.id).await.unwrap();
    assert_eq!(snapshot.state, TaskState::Killed);

    // Wait for the background process to finish after kill.
    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let notifs = mgr.pending_notifications().await;
    assert!(
        notifs.is_empty(),
        "killed task must not produce a notification"
    );
}

/// A killed task with `notified=true` still produces no notification.
#[tokio::test]
async fn test_killed_task_with_notified_produces_no_notification() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Simulate a stuck alert having fired before kill.
    mgr.mark_notified(&task.id).await;
    mgr.kill(&task.id).await.unwrap();

    let _ = wait_for_completion(&mgr, &task.id).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let notifs = mgr.pending_notifications().await;
    assert!(
        notifs.is_empty(),
        "killed task with notified=true must not produce a notification"
    );
}

// =========================================================================
// NotificationPriority — ordering and derive tests
// =========================================================================

/// Verify that `NotificationPriority` derives `Ord` with
/// `Now > Next > Later` ordering.
#[test]
fn test_notification_priority_ordering() {
    assert!(NotificationPriority::Now > NotificationPriority::Next);
    assert!(NotificationPriority::Next > NotificationPriority::Later);
    assert!(NotificationPriority::Now > NotificationPriority::Later);
}

/// Verify that `NotificationPriority` partial ordering is consistent.
#[test]
fn test_notification_priority_partial_ord_consistency() {
    assert!(
        NotificationPriority::Now.partial_cmp(&NotificationPriority::Next)
            == Some(std::cmp::Ordering::Greater)
    );
    assert!(
        NotificationPriority::Next.partial_cmp(&NotificationPriority::Later)
            == Some(std::cmp::Ordering::Greater)
    );
}

/// Verify that `NotificationPriority` variants are ordered correctly
/// when used in a `Vec::sort_by` with `Ord` derive.
#[test]
fn test_notification_priority_vec_sort() {
    let mut priorities = vec![
        NotificationPriority::Later,
        NotificationPriority::Now,
        NotificationPriority::Next,
        NotificationPriority::Later,
        NotificationPriority::Now,
    ];
    priorities.sort();
    // Ord derive: Later(0) < Next(1) < Now(2) → ascending sort
    assert_eq!(
        priorities,
        vec![
            NotificationPriority::Later,
            NotificationPriority::Later,
            NotificationPriority::Next,
            NotificationPriority::Now,
            NotificationPriority::Now,
        ]
    );
}

/// Verify `NotificationPriority` derives `Clone` and `Copy`.
#[test]
fn test_notification_priority_clone_copy() {
    let p = NotificationPriority::Now;
    let cloned = p.clone();
    let copied = p;
    assert_eq!(p, cloned);
    assert_eq!(p, copied);
}

/// Verify `NotificationPriority` derives `Serialize` / `Deserialize`.
#[test]
fn test_notification_priority_serde_roundtrip() {
    let variants = [
        NotificationPriority::Now,
        NotificationPriority::Next,
        NotificationPriority::Later,
    ];
    for v in variants {
        let json = serde_json::to_string(&v).unwrap();
        let parsed: NotificationPriority = serde_json::from_str(&json).unwrap();
        assert_eq!(v, parsed);
    }
}

/// Verify `NotificationPriority` derives `Debug`.
#[test]
fn test_notification_priority_debug() {
    assert_eq!(format!("{:?}", NotificationPriority::Now), "Now");
    assert_eq!(format!("{:?}", NotificationPriority::Next), "Next");
    assert_eq!(format!("{:?}", NotificationPriority::Later), "Later");
}
