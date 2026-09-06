//! Tests for snapshot metadata persistence in SqliteStorage.

use super::super::SqliteStorage;
use crate::persistence::{PersistenceService, SessionCheckpoint};
use crate::run_health::{SnapshotMeta, SnapshotStatus};
use chrono::Utc;

fn create_snapshot_meta(id: &str, session_id: &str) -> SnapshotMeta {
    SnapshotMeta {
        id: id.to_string(),
        reason: format!("reason-{id}"),
        created_at: Utc::now(),
        session_id: session_id.to_string(),
        status: SnapshotStatus::Pending,
    }
}

fn test_checkpoint(session_id: &str) -> SessionCheckpoint {
    SessionCheckpoint::new(session_id.to_string())
}

#[tokio::test]
async fn test_snapshot_metas_save_and_load_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();
    let session_id = "sess-sm-1";

    let metas = vec![
        create_snapshot_meta("snap-a", session_id),
        create_snapshot_meta("snap-b", session_id),
    ];

    storage
        .save_snapshot_metas(session_id, &metas)
        .await
        .unwrap();
    let loaded = storage.load_snapshot_metas(session_id).await.unwrap();

    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].id, "snap-a");
    assert_eq!(loaded[1].id, "snap-b");
}

#[tokio::test]
async fn test_snapshot_metas_empty_session() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let loaded = storage.load_snapshot_metas("nonexistent").await.unwrap();
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn test_snapshot_metas_overwrite_replaces_all() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();
    let session_id = "sess-sm-overwrite";

    // Save initial set.
    let initial = vec![create_snapshot_meta("old-1", session_id)];
    storage
        .save_snapshot_metas(session_id, &initial)
        .await
        .unwrap();

    // Overwrite with new set.
    let updated = vec![
        create_snapshot_meta("new-1", session_id),
        create_snapshot_meta("new-2", session_id),
    ];
    storage
        .save_snapshot_metas(session_id, &updated)
        .await
        .unwrap();

    let loaded = storage.load_snapshot_metas(session_id).await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.iter().all(|m| m.id.starts_with("new-")));
}

#[tokio::test]
async fn test_snapshot_metas_independent_of_checkpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();
    let session_id = "sess-sm-ind";

    // Save a checkpoint with empty snapshot_metas.
    storage
        .save_checkpoint(&test_checkpoint(session_id))
        .await
        .unwrap();

    // Save snapshot metas.
    let metas = vec![create_snapshot_meta("snap-ind", session_id)];
    storage
        .save_snapshot_metas(session_id, &metas)
        .await
        .unwrap();

    // Checkpoint snapshot_metas should remain empty.
    let cp = storage.load_checkpoint(session_id).await.unwrap().unwrap();
    assert!(cp.snapshot_metas.is_empty());

    // Independent store should have the metadata.
    let loaded = storage.load_snapshot_metas(session_id).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "snap-ind");
}

#[tokio::test]
async fn test_snapshot_metas_status_persists() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();
    let session_id = "sess-sm-status";

    let mut meta = create_snapshot_meta("snap-status", session_id);
    meta.status = SnapshotStatus::Complete;
    storage
        .save_snapshot_metas(session_id, &[meta])
        .await
        .unwrap();

    let loaded = storage.load_snapshot_metas(session_id).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, SnapshotStatus::Complete);
}

#[tokio::test]
async fn test_snapshot_metas_session_id_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = SqliteStorage::new(tmp.path()).unwrap();

    let meta_a = create_snapshot_meta("snap-a", "sess-A");
    let meta_b = create_snapshot_meta("snap-b", "sess-B");
    storage
        .save_snapshot_metas("sess-A", &[meta_a])
        .await
        .unwrap();
    storage
        .save_snapshot_metas("sess-B", &[meta_b])
        .await
        .unwrap();

    let loaded_a = storage.load_snapshot_metas("sess-A").await.unwrap();
    let loaded_b = storage.load_snapshot_metas("sess-B").await.unwrap();
    assert_eq!(loaded_a.len(), 1);
    assert_eq!(loaded_a[0].id, "snap-a");
    assert_eq!(loaded_b.len(), 1);
    assert_eq!(loaded_b[0].id, "snap-b");
}
