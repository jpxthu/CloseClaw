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
            if snapshot.state != TaskState::Running {
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
    let task = mgr.spawn("echo hello", _tmp.path()).await.unwrap();
    assert_eq!(task.state, TaskState::Running);
}

// ---------------------------------------------------------------------------
// 2. task completes successfully
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_completes() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    assert_eq!(snapshot.state, TaskState::Completed { exit_code: 0 });
}

// ---------------------------------------------------------------------------
// 3. task fails
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_fails() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("false", _tmp.path()).await.unwrap();
    let snapshot = wait_for_completion(&mgr, &task.id).await;
    assert_eq!(snapshot.state, TaskState::Failed { exit_code: 1 });
}

#[tokio::test]
async fn test_spawn_nonexistent_command() {
    let (mgr, _tmp) = test_manager();
    let task = mgr
        .spawn("nonexistent_cmd_xyz_12345", _tmp.path())
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
    let task = mgr.spawn("sleep 60", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("echo hello", _tmp.path()).await.unwrap();
    let snapshot = mgr.get_task(&task.id).await;
    assert!(snapshot.is_some());
    let s = snapshot.unwrap();
    assert_eq!(s.id, task.id);
    assert_eq!(s.command, "echo hello");
    assert_eq!(s.state, TaskState::Running);
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
    let task = mgr.spawn("echo hello_output", _tmp.path()).await.unwrap();
    let _ = wait_for_completion(&mgr, &task.id).await;
    let content = tokio::fs::read_to_string(&task.output_path).await.unwrap();
    assert!(content.contains("hello_output"));
}

#[tokio::test]
async fn test_output_file_captures_stderr() {
    let (mgr, _tmp) = test_manager();
    let task = mgr
        .spawn("echo hello_stderr >&2", _tmp.path())
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
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("false", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
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
    let task = mgr.spawn("true", _tmp.path()).await.unwrap();
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
        .backgroundize(child, "true")
        .await
        .expect("backgroundize(child, command) should succeed");

    assert_eq!(task.state, TaskState::Running);
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
        .backgroundize(child, "sleep 60")
        .await
        .expect("backgroundize should accept a long-running child");
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
        .backgroundize(child, "echo bgize_output")
        .await
        .expect("backgroundize should succeed");
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
