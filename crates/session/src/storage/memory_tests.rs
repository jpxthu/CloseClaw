//! Unit tests for PersistenceService new methods added in Step 1.1.
//!
//! Tests `list_archived_unmined_sessions`, `list_mined_undreamt_sessions`,
//! `mark_mined`, and `update_dreaming_status` on MemoryStorage.

use crate::persistence::{DreamingStatus, PersistenceService, SessionCheckpoint};
use crate::storage::memory::MemoryStorage;

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_checkpoint(session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint::new(session_id.into())
}

// ── list_archived_unmined_sessions ───────────────────────────────────────

#[tokio::test]
async fn test_list_archived_unmined_sessions_filters_correctly() {
    let storage = MemoryStorage::new();

    // Archived, unmined → should appear.
    let mut cp1 = make_checkpoint("archived-unmined");
    cp1.mined = false;
    storage.archive_checkpoint(&cp1).await.unwrap();

    // Archived, mined → should NOT appear.
    let mut cp2 = make_checkpoint("archived-mined");
    cp2.mined = true;
    storage.archive_checkpoint(&cp2).await.unwrap();

    // Active, unmined → should NOT appear (not archived).
    let mut cp3 = make_checkpoint("active-unmined");
    cp3.mined = false;
    storage.save_checkpoint(&cp3).await.unwrap();

    let result = storage.list_archived_unmined_sessions().await.unwrap();
    assert_eq!(result.len(), 1, "only archived-unmined should appear");
    assert!(result.contains(&"archived-unmined".to_string()));
}

#[tokio::test]
async fn test_list_archived_unmined_sessions_empty_storage() {
    let storage = MemoryStorage::new();
    let result = storage.list_archived_unmined_sessions().await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_list_archived_unmined_sessions_all_mined() {
    let storage = MemoryStorage::new();

    let mut cp1 = make_checkpoint("s1");
    cp1.mined = true;
    storage.archive_checkpoint(&cp1).await.unwrap();

    let mut cp2 = make_checkpoint("s2");
    cp2.mined = true;
    storage.archive_checkpoint(&cp2).await.unwrap();

    let result = storage.list_archived_unmined_sessions().await.unwrap();
    assert!(result.is_empty(), "all mined sessions should be excluded");
}

// ── list_mined_undreamt_sessions ─────────────────────────────────────────

#[tokio::test]
async fn test_list_mined_undreamt_sessions_filters_correctly() {
    let storage = MemoryStorage::new();

    // Mined, Pending → should appear.
    let mut cp1 = make_checkpoint("mined-pending");
    cp1.mined = true;
    cp1.dreaming_status = DreamingStatus::Pending;
    storage.save_checkpoint(&cp1).await.unwrap();

    // Mined, Completed → should NOT appear.
    let mut cp2 = make_checkpoint("mined-completed");
    cp2.mined = true;
    cp2.dreaming_status = DreamingStatus::Completed;
    storage.save_checkpoint(&cp2).await.unwrap();

    // Not mined, Pending → should NOT appear.
    let mut cp3 = make_checkpoint("unmined-pending");
    cp3.mined = false;
    cp3.dreaming_status = DreamingStatus::Pending;
    storage.save_checkpoint(&cp3).await.unwrap();

    let result = storage.list_mined_undreamt_sessions().await.unwrap();
    assert_eq!(result.len(), 1, "only mined-pending should appear");
    assert!(result.contains(&"mined-pending".to_string()));
}

#[tokio::test]
async fn test_list_mined_undreamt_sessions_empty_storage() {
    let storage = MemoryStorage::new();
    let result = storage.list_mined_undreamt_sessions().await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn test_list_mined_undreamt_sessions_all_completed() {
    let storage = MemoryStorage::new();

    let mut cp = make_checkpoint("s-done");
    cp.mined = true;
    cp.dreaming_status = DreamingStatus::Completed;
    storage.save_checkpoint(&cp).await.unwrap();

    let result = storage.list_mined_undreamt_sessions().await.unwrap();
    assert!(result.is_empty());
}

// ── mark_mined ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_mark_mined_updates_active_checkpoint() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("active-to-mined");
    cp.mined = false;
    storage.save_checkpoint(&cp).await.unwrap();

    storage.mark_mined("active-to-mined").await.unwrap();

    let loaded = storage
        .load_checkpoint("active-to-mined")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.mined, "active checkpoint should be marked mined");
}

#[tokio::test]
async fn test_mark_mined_updates_archived_checkpoint() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("archived-to-mined");
    cp.mined = false;
    storage.archive_checkpoint(&cp).await.unwrap();

    storage.mark_mined("archived-to-mined").await.unwrap();

    let loaded = storage
        .restore_checkpoint("archived-to-mined")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.mined, "archived checkpoint should be marked mined");
}

#[tokio::test]
async fn test_mark_mined_nonexistent_is_noop() {
    let storage = MemoryStorage::new();
    // Should not error.
    storage.mark_mined("nonexistent").await.unwrap();
}

// ── mined_at ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_mark_mined_sets_mined_at_on_active() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("mined-at-active");
    cp.mined = false;
    assert!(cp.mined_at.is_none(), "mined_at should start as None");
    storage.save_checkpoint(&cp).await.unwrap();

    let before = chrono::Utc::now().timestamp();
    storage.mark_mined("mined-at-active").await.unwrap();
    let after = chrono::Utc::now().timestamp();

    let loaded = storage
        .load_checkpoint("mined-at-active")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.mined, "should be marked mined");
    let ts = loaded
        .mined_at
        .expect("mined_at should be Some after mark_mined");
    assert!(
        ts >= before && ts <= after,
        "mined_at ({ts}) should be between {before} and {after}"
    );
}

#[tokio::test]
async fn test_mark_mined_sets_mined_at_on_archived() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("mined-at-archived");
    cp.mined = false;
    storage.archive_checkpoint(&cp).await.unwrap();

    let before = chrono::Utc::now().timestamp();
    storage.mark_mined("mined-at-archived").await.unwrap();
    let after = chrono::Utc::now().timestamp();

    let loaded = storage
        .restore_checkpoint("mined-at-archived")
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.mined);
    let ts = loaded
        .mined_at
        .expect("mined_at should be Some after mark_mined");
    assert!(
        ts >= before && ts <= after,
        "mined_at ({ts}) should be between {before} and {after}"
    );
}

#[tokio::test]
async fn test_mined_at_defaults_none_before_mark() {
    let storage = MemoryStorage::new();
    let cp = make_checkpoint("mined-at-default");
    assert!(cp.mined_at.is_none());
    storage.save_checkpoint(&cp).await.unwrap();

    let loaded = storage
        .load_checkpoint("mined-at-default")
        .await
        .unwrap()
        .unwrap();
    assert!(
        loaded.mined_at.is_none(),
        "mined_at should remain None before mark_mined"
    );
}

// ── update_dreaming_status ───────────────────────────────────────────────

#[tokio::test]
async fn test_update_dreaming_status_on_active_checkpoint() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("dream-status-active");
    cp.dreaming_status = DreamingStatus::Pending;
    storage.save_checkpoint(&cp).await.unwrap();

    storage
        .update_dreaming_status("dream-status-active", DreamingStatus::InLight)
        .await
        .unwrap();

    let loaded = storage
        .load_checkpoint("dream-status-active")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.dreaming_status, DreamingStatus::InLight);
}

#[tokio::test]
async fn test_update_dreaming_status_on_archived_checkpoint() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("dream-status-archived");
    cp.dreaming_status = DreamingStatus::Pending;
    storage.archive_checkpoint(&cp).await.unwrap();

    storage
        .update_dreaming_status("dream-status-archived", DreamingStatus::InDeep)
        .await
        .unwrap();

    let loaded = storage
        .restore_checkpoint("dream-status-archived")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.dreaming_status, DreamingStatus::InDeep);
}

#[tokio::test]
async fn test_update_dreaming_status_nonexistent_is_noop() {
    let storage = MemoryStorage::new();
    // Should not error.
    storage
        .update_dreaming_status("nonexistent", DreamingStatus::Completed)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_update_dreaming_status_full_lifecycle() {
    let storage = MemoryStorage::new();
    let mut cp = make_checkpoint("lifecycle");
    cp.dreaming_status = DreamingStatus::Pending;
    storage.save_checkpoint(&cp).await.unwrap();

    let stages = [
        DreamingStatus::InLight,
        DreamingStatus::InRem,
        DreamingStatus::InDeep,
        DreamingStatus::Completed,
    ];

    for stage in &stages {
        storage
            .update_dreaming_status("lifecycle", *stage)
            .await
            .unwrap();
        let loaded = storage.load_checkpoint("lifecycle").await.unwrap().unwrap();
        assert_eq!(loaded.dreaming_status, *stage);
    }
}
