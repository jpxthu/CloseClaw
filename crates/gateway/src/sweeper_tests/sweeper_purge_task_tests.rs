//! purge + TaskManager integration tests: terminal task output cleanup on
//! purge, with session-level isolation.

use closeclaw_session::persistence::PersistenceService;
use closeclaw_tasks::TaskManager;
use std::sync::Arc;

use crate::sweeper::ArchiveSweeper;

use super::sweeper_test_utils::{MemStorage, MockTaskManager};

// ── purge + TaskManager integration ────────────────────────────────

/// When `purge_and_invalidate_impl` is called with a TaskManager,
/// `cleanup_all_finished()` is invoked to remove all terminal task
/// output files, and the correct `session_id` is passed.
#[tokio::test]
async fn test_purge_and_invalidate_calls_cleanup_all_finished() {
    let mem = Arc::new(MemStorage::default());
    mem.add_expired_session("purge-with-tm".into());
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;

    let (tm, called_flag, sid_arg) = MockTaskManager::new();
    let tm_ref: Arc<dyn TaskManager> = Arc::new(tm);

    ArchiveSweeper::purge_and_invalidate_impl(
        Arc::clone(&storage),
        "purge-with-tm".into(),
        Some(tm_ref.as_ref()),
    )
    .await
    .unwrap();

    assert!(
        *called_flag.lock().unwrap(),
        "cleanup_all_finished must be called when task_manager is provided"
    );
    assert_eq!(
        sid_arg.lock().unwrap().as_deref(),
        Some("purge-with-tm"),
        "cleanup_all_finished must receive the correct session_id"
    );
    let purge_called = mem.purge_called.lock().unwrap();
    assert!(purge_called.contains(&"purge-with-tm".into()));
}

/// Purging session A must not invoke cleanup for session B,
/// verifying session-level isolation of task output cleanup.
#[tokio::test]
async fn test_purge_session_a_does_not_affect_session_b() {
    let mem = Arc::new(MemStorage::default());
    mem.add_expired_session("session-a".into());
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;

    let (tm, _, sid_arg) = MockTaskManager::new();
    let tm_ref: Arc<dyn TaskManager> = Arc::new(tm);

    ArchiveSweeper::purge_and_invalidate_impl(
        Arc::clone(&storage),
        "session-a".into(),
        Some(tm_ref.as_ref()),
    )
    .await
    .unwrap();

    assert_eq!(
        sid_arg.lock().unwrap().as_deref(),
        Some("session-a"),
        "cleanup_all_finished must receive session-a, not session-b"
    );
}

/// When `purge_and_invalidate_impl` is called without a TaskManager,
/// no cleanup is attempted (graceful no-op).
#[tokio::test]
async fn test_purge_and_invalidate_without_task_manager_skips_cleanup() {
    let mem = Arc::new(MemStorage::default());
    mem.add_expired_session("purge-no-tm".into());
    let storage: Arc<dyn PersistenceService> = mem.clone() as _;

    ArchiveSweeper::purge_and_invalidate_impl(Arc::clone(&storage), "purge-no-tm".into(), None)
        .await
        .unwrap();

    let purge_called = mem.purge_called.lock().unwrap();
    assert!(purge_called.contains(&"purge-no-tm".into()));
}
