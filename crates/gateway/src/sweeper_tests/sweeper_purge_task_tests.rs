//! purge + TaskManager integration tests: terminal task output cleanup on
//! purge, with session-level isolation.

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
    let storage = mem.storage_arc();

    let tm = Arc::new(MockTaskManager::new());

    ArchiveSweeper::purge_and_invalidate_impl(
        Arc::clone(&storage),
        "purge-with-tm".into(),
        Some(tm.as_ref()),
    )
    .await
    .unwrap();

    assert!(
        tm.was_called(),
        "cleanup_all_finished must be called when task_manager is provided"
    );
    assert_eq!(
        tm.last_session_id().as_deref(),
        Some("purge-with-tm"),
        "cleanup_all_finished must receive the correct session_id"
    );
    let purge_called = mem.purge_called.lock().unwrap();
    assert!(
        purge_called.contains(&"purge-with-tm".into()),
        "purge must be called for purge-with-tm; actual purge calls: {purge_called:?}"
    );
}

/// Purging expired session A must be isolated: `cleanup_all_finished`
/// receives exactly session-a, and independent session-b — not expired,
/// with its own persisted checkpoint — is never cleaned or purged, and its
/// persisted data survives.
#[tokio::test]
async fn test_purge_session_a_does_not_affect_session_b() {
    let mem = Arc::new(MemStorage::default());
    mem.add_expired_session("session-a".into());
    mem.add_root("session-a");
    mem.add_root("session-b");
    let storage = mem.storage_arc();

    let tm = Arc::new(MockTaskManager::new());

    ArchiveSweeper::purge_and_invalidate_impl(
        Arc::clone(&storage),
        "session-a".into(),
        Some(tm.as_ref()),
    )
    .await
    .unwrap();

    let purge_called = mem.purge_called.lock().unwrap().clone();
    assert_eq!(
        purge_called,
        vec!["session-a".to_owned()],
        "only expired session-a must be purged; actual purge calls: {purge_called:?}"
    );
    let cleanup_calls = tm.called_session_ids();
    assert_eq!(
        cleanup_calls,
        vec!["session-a".to_owned()],
        "cleanup_all_finished must be invoked for session-a only; actual calls: {cleanup_calls:?}"
    );
    storage
        .load_checkpoint("session-b")
        .await
        .unwrap()
        .expect("session-b checkpoint must survive purging session-a");
    let deleted = mem.deleted_ids();
    assert!(
        deleted.is_empty(),
        "purging session-a must not delete any session; actual deleted ids: {deleted:?}"
    );
}

/// When `purge_and_invalidate_impl` is called without a TaskManager,
/// no cleanup is attempted (graceful no-op).
#[tokio::test]
async fn test_purge_and_invalidate_without_task_manager_skips_cleanup() {
    let mem = Arc::new(MemStorage::default());
    mem.add_expired_session("purge-no-tm".into());
    let storage = mem.storage_arc();

    ArchiveSweeper::purge_and_invalidate_impl(Arc::clone(&storage), "purge-no-tm".into(), None)
        .await
        .unwrap();

    let purge_called = mem.purge_called.lock().unwrap();
    assert!(
        purge_called.contains(&"purge-no-tm".into()),
        "purge must be called for purge-no-tm (no TaskManager); actual: {purge_called:?}"
    );
}
