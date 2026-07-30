//! Tests for cleanup_finished — output preservation & edge cases.

use super::*;
use tempfile::TempDir;

fn test_manager() -> (BackgroundTaskManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let mgr = BackgroundTaskManager::with_temp_dir(tmp.path());
    (mgr, tmp)
}

async fn insert_handle(
    mgr: &BackgroundTaskManager,
    task_id: &str,
    command: &str,
    state: TaskState,
) -> std::path::PathBuf {
    let tmp = mgr.temp_dir.join("closeclaw/background").join(task_id);
    let output_path = tmp.join("output");
    tokio::fs::create_dir_all(&tmp).await.unwrap();
    tokio::fs::write(&output_path, "test output").await.unwrap();
    let handle = TaskHandle {
        id: task_id.to_owned(),
        command: command.to_owned(),
        state,
        output_path: output_path.clone(),
        kill_tx: None,
        notified: false,
        created_at: tokio::time::Instant::now(),
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
    mgr.tasks.lock().await.insert(
        "io-err".to_owned(),
        TaskHandle {
            id: "io-err".to_owned(),
            command: "test".to_owned(),
            state: TaskState::Completed { exit_code: 0 },
            output_path: output,
            kill_tx: None,
            notified: false,
            created_at: tokio::time::Instant::now(),
        },
    );
    // Should not panic — remove_dir_all on a missing path logs a warning
    mgr.cleanup_finished().await;
    assert!(mgr.get_task("io-err").await.is_none());
}
