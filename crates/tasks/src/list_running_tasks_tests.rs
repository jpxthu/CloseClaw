//! Tests for `list_running_tasks()`.
//!
//! Split from `background_tests.rs` (Step 1.5) to keep file sizes under the
//! 1000-line limit.

use super::*;
use crate::TaskManager;
use std::time::Duration;
use tempfile::TempDir;

fn test_manager() -> (BackgroundTaskManager, TempDir) {
    let tmp = TempDir::new().unwrap();
    let mgr = BackgroundTaskManager::with_temp_dir(tmp.path());
    (mgr, tmp)
}

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
// list_running_tasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_running_empty() {
    let (mgr, _tmp) = test_manager();
    assert!(mgr.list_running_tasks().await.is_empty());
}

#[tokio::test]
async fn test_list_running_returns_correct_info() {
    let (mgr, _tmp) = test_manager();
    let task = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    let r = mgr.list_running_tasks().await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].task_id, task.id);
    assert_eq!(r[0].command, "sleep 60");
    // Step 1.5: verify elapsed_secs is reasonable (task just spawned)
    assert!(
        r[0].elapsed_secs <= 1,
        "elapsed_secs should be <= 1 for a just-spawned task, got {}",
        r[0].elapsed_secs
    );
    mgr.kill(&task.id).await.unwrap();
}

#[tokio::test]
async fn test_list_running_excludes_completed() {
    let (mgr, _tmp) = test_manager();
    let fast = mgr.spawn("true", _tmp.path(), false).await.unwrap();
    let slow = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    let _ = wait_for_completion(&mgr, &fast.id).await;
    let r = mgr.list_running_tasks().await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].task_id, slow.id);
    mgr.kill(&slow.id).await.unwrap();
}

// ---------------------------------------------------------------------------
// Step 1.5: two running tasks simultaneously
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_running_two_tasks() {
    let (mgr, _tmp) = test_manager();
    let task1 = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    let task2 = mgr.spawn("sleep 60", _tmp.path(), false).await.unwrap();
    let r = mgr.list_running_tasks().await;
    assert_eq!(r.len(), 2, "should list both running tasks");

    let ids: Vec<&str> = r.iter().map(|i| i.task_id.as_str()).collect();
    assert!(
        ids.contains(&task1.id.as_str()),
        "missing task1 in running list"
    );
    assert!(
        ids.contains(&task2.id.as_str()),
        "missing task2 in running list"
    );

    mgr.kill(&task1.id).await.unwrap();
    mgr.kill(&task2.id).await.unwrap();
}
