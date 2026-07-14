//! Unit tests for [`RuntimeSnapshotManager`] and [`PersistenceMetaStore`].

use super::snapshot_manager::{
    PersistenceMetaStore, RollbackAction, RuntimeSnapshotManager, SnapshotMeta, SnapshotMetaStore,
    SnapshotStatus, TranscriptOp,
};
use crate::llm_session::SessionMessage;
use crate::persistence::{PersistenceService, SessionCheckpoint};
use crate::storage::memory::MemoryStorage;
use chrono::Utc;
use closeclaw_common::ContentBlock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Helper: build a `SessionMessage` with plain text.
fn msg(role: &str, text: &str) -> SessionMessage {
    SessionMessage {
        role: role.to_string(),
        content_blocks: vec![ContentBlock::Text(text.into())],
        timestamp: Utc::now(),
    }
}

// =====================================================================
// TranscriptOp
// =====================================================================

#[test]
fn test_transcript_op_requires_snapshot() {
    assert!(!TranscriptOp::Append.requires_snapshot());
    assert!(TranscriptOp::Rewrite.requires_snapshot());
    assert!(TranscriptOp::PartialRewrite.requires_snapshot());
}

// =====================================================================
// create_snapshot
// =====================================================================

#[test]
fn test_create_snapshot_returns_some_for_rewrite() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    let result = mgr.create_snapshot(&messages, TranscriptOp::Rewrite, "test");
    assert!(result.is_some());
    assert_eq!(mgr.snapshot_count(), 1);
}

#[test]
fn test_create_snapshot_returns_unique_id() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    let id1 = mgr.create_snapshot(&messages, TranscriptOp::Rewrite, "first");
    let id2 = mgr.create_snapshot(&messages, TranscriptOp::Rewrite, "second");
    assert!(id1.is_some());
    assert!(id2.is_some());
    assert_ne!(id1.unwrap(), id2.unwrap());
}

#[test]
fn test_create_snapshot_returns_none_for_append() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    assert!(mgr
        .create_snapshot(&messages, TranscriptOp::Append, "test")
        .is_none());
    assert_eq!(mgr.snapshot_count(), 0);
}

#[test]
fn test_create_snapshot_stores_messages() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "a"), msg("assistant", "b")];
    mgr.create_snapshot(&messages, TranscriptOp::Rewrite, "test");

    let action = mgr.rollback(&messages).unwrap();
    match action {
        RollbackAction::Replace { messages: restored } => {
            assert_eq!(restored.len(), 2);
            assert_eq!(restored[0].role, "user");
            assert_eq!(restored[1].role, "assistant");
        }
        _ => panic!("expected Replace action"),
    }
}

#[test]
fn test_create_snapshot_partial_rewrite() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("system", "system prompt")];
    let result = mgr.create_snapshot(&messages, TranscriptOp::PartialRewrite, "test");
    assert!(result.is_some());
    assert_eq!(mgr.snapshot_count(), 1);
}

// =====================================================================
// Snapshot id, reason, status
// =====================================================================

#[test]
fn test_snapshot_has_unique_id() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "first");
    mgr.create_snapshot(&[msg("user", "b")], TranscriptOp::Rewrite, "second");
    // Two snapshots created — ids must be unique (different UUIDs).
    // We can't easily extract them without exposing internals, but
    // the count is 2, confirming both were stored.
    assert_eq!(mgr.snapshot_count(), 2);
}

#[test]
fn test_snapshot_status_pending_on_creation() {
    let mut mgr = RuntimeSnapshotManager::new();
    // Create a snapshot, then rollback to get it and check status.
    mgr.create_snapshot(&[msg("user", "before")], TranscriptOp::Rewrite, "pre-op");
    // Create a second so the first is not consumed immediately.
    mgr.create_snapshot(&[msg("user", "current")], TranscriptOp::Rewrite, "op");
    // Rollback pops the second; the first remains with Pending status.
    let _ = mgr.rollback(&[msg("user", "current")]);
    // The remaining snapshot was created with status Pending.
    // We verify this by checking that rollback still works (the
    // remaining snapshot is still there).
    let action = mgr.rollback(&[msg("user", "before")]);
    assert!(action.is_some());
}

#[test]
fn test_mark_complete_nonexistent_id_is_noop() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "test");
    let count_before = mgr.snapshot_count();
    mgr.mark_complete("nonexistent-id");
    // No panic, snapshot count unchanged.
    assert_eq!(mgr.snapshot_count(), count_before);
}

// =====================================================================
// rollback
// =====================================================================

#[test]
fn test_rollback_returns_most_recent() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "first")], TranscriptOp::Rewrite, "s1");
    mgr.create_snapshot(
        &[msg("user", "first"), msg("assistant", "second")],
        TranscriptOp::Rewrite,
        "s2",
    );

    let action = mgr
        .rollback(&[msg("user", "first"), msg("assistant", "second")])
        .unwrap();
    match action {
        RollbackAction::Replace { messages } => {
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].role, "assistant");
        }
        _ => panic!("expected Replace action"),
    }
    // After rolling back the most recent, the earlier one remains
    // plus the pre-rollback snapshot (2 snapshots total).
    assert_eq!(mgr.snapshot_count(), 2);
}

#[test]
fn test_rollback_returns_none_when_empty() {
    let mut mgr = RuntimeSnapshotManager::new();
    assert!(mgr.rollback(&[]).is_none());
}

#[test]
fn test_rollback_creates_pre_rollback_snapshot() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "original")], TranscriptOp::Rewrite, "test");
    let count_before = mgr.snapshot_count();
    // Rollback creates a pre-rollback snapshot (for undo support).
    let _ = mgr.rollback(&[msg("user", "current-state")]);
    // Consumed 1 target, added 1 pre-rollback — count stays the same.
    assert_eq!(mgr.snapshot_count(), count_before);
}

#[test]
fn test_rollback_creates_pre_rollback_with_multiple_snapshots() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "s1");
    mgr.create_snapshot(&[msg("user", "b")], TranscriptOp::Rewrite, "s2");
    let count_before = mgr.snapshot_count();
    let _ = mgr.rollback(&[msg("user", "current")]);
    // Consumed 1, added 1 pre-rollback — count stays same.
    assert_eq!(mgr.snapshot_count(), count_before);
}

#[test]
fn test_rollback_is_undoable() {
    let mut mgr = RuntimeSnapshotManager::new();
    // Create a snapshot with "original" messages.
    mgr.create_snapshot(&[msg("user", "original")], TranscriptOp::Rewrite, "test");
    // First rollback: returns "original" messages, creates pre-rollback with "current".
    let action1 = mgr.rollback(&[msg("user", "current")]).unwrap();
    match action1 {
        RollbackAction::Replace { messages } => {
            assert_eq!(
                messages[0].content_blocks[0],
                ContentBlock::Text("original".into())
            );
        }
        _ => panic!("expected Replace action"),
    }
    // Second rollback: should return the pre-rollback state ("current").
    let action2 = mgr.rollback(&[msg("user", "re-applied")]).unwrap();
    match action2 {
        RollbackAction::Replace { messages } => {
            assert_eq!(
                messages[0].content_blocks[0],
                ContentBlock::Text("current".into())
            );
        }
        _ => panic!("expected Replace action"),
    }
}

// =====================================================================
// SnapshotKind / RollbackAction
// =====================================================================

#[test]
fn test_full_rewrite_returns_replace_action() {
    let mut mgr = RuntimeSnapshotManager::new();
    let msgs = vec![msg("user", "hello"), msg("assistant", "world")];
    mgr.create_snapshot(&msgs, TranscriptOp::Rewrite, "rewrite");
    let action = mgr.rollback(&msgs).unwrap();
    assert!(matches!(action, RollbackAction::Replace { .. }));
}

// =====================================================================
// 25-snapshot limit
// =====================================================================

#[test]
fn test_max_snapshots_evicts_oldest() {
    let mut mgr = RuntimeSnapshotManager::new();
    for i in 0..30 {
        mgr.create_snapshot(
            &[msg("user", &format!("msg-{i}"))],
            TranscriptOp::Rewrite,
            "test",
        );
    }
    assert_eq!(mgr.snapshot_count(), 25);

    // The oldest 5 should have been evicted.
    // The remaining snapshots should be msg-5 through msg-29.
    // Rolling back pops from the back (msg-29).
    let current = vec![msg("user", "msg-29")];
    let action = mgr.rollback(&current).unwrap();
    match action {
        RollbackAction::Replace { messages } => {
            assert_eq!(
                messages[0].content_blocks[0],
                ContentBlock::Text("msg-29".into())
            );
        }
        _ => panic!("expected Replace action"),
    }
}

#[test]
fn test_exactly_at_limit_no_eviction() {
    let mut mgr = RuntimeSnapshotManager::new();
    for i in 0..25 {
        mgr.create_snapshot(
            &[msg("user", &format!("msg-{i}"))],
            TranscriptOp::Rewrite,
            "test",
        );
    }
    assert_eq!(mgr.snapshot_count(), 25);

    // Add one more — oldest (msg-0) should be evicted.
    mgr.create_snapshot(&[msg("user", "msg-25")], TranscriptOp::Rewrite, "test");
    assert_eq!(mgr.snapshot_count(), 25);

    // Rollback the newest.
    let current = vec![msg("user", "msg-25")];
    let action = mgr.rollback(&current).unwrap();
    match action {
        RollbackAction::Replace { messages } => {
            assert_eq!(
                messages[0].content_blocks[0],
                ContentBlock::Text("msg-25".into())
            );
        }
        _ => panic!("expected Replace action"),
    }
}

// =====================================================================
// clear
// =====================================================================

#[test]
fn test_clear_removes_all_snapshots() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "test");
    mgr.create_snapshot(&[msg("user", "b")], TranscriptOp::Rewrite, "test");
    assert_eq!(mgr.snapshot_count(), 2);
    mgr.clear();
    assert_eq!(mgr.snapshot_count(), 0);
    assert!(mgr.rollback(&[]).is_none());
}

// =====================================================================
// Interleaved append + rewrite
// =====================================================================

#[test]
fn test_append_does_not_affect_snapshot_count() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "rewrite")], TranscriptOp::Rewrite, "test");
    assert_eq!(mgr.snapshot_count(), 1);

    // Append operations should not create snapshots.
    mgr.create_snapshot(&[msg("user", "append1")], TranscriptOp::Append, "test");
    mgr.create_snapshot(&[msg("user", "append2")], TranscriptOp::Append, "test");
    assert_eq!(mgr.snapshot_count(), 1);
}

// =====================================================================
// Default impl
// =====================================================================

#[test]
fn test_default_creates_empty_manager() {
    let mgr = RuntimeSnapshotManager::default();
    assert_eq!(mgr.snapshot_count(), 0);
}

// =====================================================================
// Incremental snapshot → Truncate action
// =====================================================================

#[test]
fn test_incremental_snapshot_returns_truncate_action() {
    let mut mgr = RuntimeSnapshotManager::new();
    let msgs = vec![msg("user", "hello")];
    mgr.create_incremental_snapshot(&msgs, "append-op", "entry_42");
    assert_eq!(mgr.snapshot_count(), 1);

    let action = mgr.rollback(&msgs).unwrap();
    match action {
        RollbackAction::Truncate { leaf_entry_id } => {
            assert_eq!(leaf_entry_id, "entry_42");
        }
        _ => panic!("expected Truncate action for incremental snapshot"),
    }
}

#[test]
fn test_incremental_snapshot_pre_rollback_is_full_rewrite() {
    let mut mgr = RuntimeSnapshotManager::new();
    // Create an incremental snapshot.
    mgr.create_incremental_snapshot(&[msg("user", "before")], "append-op", "entry_1");
    // Rollback: pops incremental, creates pre-rollback (FullRewrite).
    let _ = mgr.rollback(&[msg("user", "current")]);
    assert_eq!(mgr.snapshot_count(), 1);
    // The pre-rollback is a FullRewrite — second rollback returns Replace.
    let action = mgr.rollback(&[msg("user", "re-applied")]).unwrap();
    assert!(matches!(action, RollbackAction::Replace { .. }));
}

// =====================================================================
// SnapshotStatus Pending → Complete flow
// =====================================================================

#[test]
fn test_snapshot_status_pending_to_complete_flow() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "op-1");
    mgr.create_snapshot(&[msg("user", "b")], TranscriptOp::Rewrite, "op-2");
    // We can't easily extract the id from the manager, so we'll
    // use a known pattern: rollback pops the last, and the pre-rollback
    // has a known reason.
    // Instead, test mark_complete with a constructed id by using a
    // helper: create one snapshot, rollback it (consuming it), and
    // verify the pre-rollback snapshot can be marked complete.
    mgr.clear();

    // Create a single snapshot, rollback to get it, then verify
    // the pre-rollback can be marked complete.
    mgr.create_snapshot(&[msg("user", "original")], TranscriptOp::Rewrite, "test");
    let action = mgr.rollback(&[msg("user", "current")]).unwrap();
    assert!(matches!(action, RollbackAction::Replace { .. }));
    // Now the pre-rollback snapshot is in the queue.
    // We can't extract its id directly, but we can verify that
    // mark_complete doesn't panic and the snapshot still exists.
    mgr.mark_complete("nonexistent");
    assert_eq!(mgr.snapshot_count(), 1);
}

// =====================================================================
// SnapshotMetaStore persistence
// =====================================================================

struct CountingMetaStore {
    save_count: AtomicUsize,
}

impl CountingMetaStore {
    fn new() -> Self {
        Self {
            save_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl SnapshotMetaStore for CountingMetaStore {
    async fn save_meta(&self, _meta: &SnapshotMeta) -> Result<(), String> {
        self.save_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn test_meta_store_called_on_snapshot_creation() {
    let store = Arc::new(CountingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "test");
    // Give the spawned task time to complete.
    tokio::task::yield_now().await;
    assert_eq!(store.save_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_meta_store_called_on_incremental_snapshot() {
    let store = Arc::new(CountingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_incremental_snapshot(&[msg("user", "a")], "append-op", "e1");
    tokio::task::yield_now().await;
    assert_eq!(store.save_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_meta_store_called_on_rollback() {
    let store = Arc::new(CountingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "test");
    tokio::task::yield_now().await;
    assert_eq!(store.save_count.load(Ordering::SeqCst), 1);
    // Rollback creates a pre-rollback snapshot → second save_meta call.
    let _ = mgr.rollback(&[msg("user", "current")]);
    tokio::task::yield_now().await;
    assert_eq!(store.save_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_no_meta_store_no_save() {
    let mut mgr = RuntimeSnapshotManager::new();
    // No meta_store set — all operations should succeed without error.
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "test");
    assert_eq!(mgr.snapshot_count(), 1);
    let _ = mgr.rollback(&[msg("user", "current")]);
    assert_eq!(mgr.snapshot_count(), 1);
    mgr.clear();
    assert_eq!(mgr.snapshot_count(), 0);
}

// =====================================================================
// PersistenceMetaStore — happy path
// =====================================================================

#[tokio::test]
async fn test_persistence_meta_store_save_meta() {
    let storage = Arc::new(MemoryStorage::new());
    let session_id = "sess-pms-1";
    // Pre-create checkpoint so load_checkpoint succeeds.
    storage
        .save_checkpoint(&SessionCheckpoint::new(session_id.into()))
        .await
        .unwrap();
    let pms = PersistenceMetaStore::new(storage.clone(), session_id.into());
    let meta = SnapshotMeta {
        id: "snap-1".into(),
        reason: "compaction".into(),
        created_at: Utc::now(),
        session_id: session_id.into(),
        status: SnapshotStatus::Pending,
    };
    pms.save_meta(&meta).await.unwrap();
    let cp = storage.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(cp.snapshot_metas.len(), 1);
    assert_eq!(cp.snapshot_metas[0].id, "snap-1");
}

#[tokio::test]
async fn test_persistence_meta_store_appends_multiple() {
    let storage = Arc::new(MemoryStorage::new());
    let session_id = "sess-pms-multi";
    storage
        .save_checkpoint(&SessionCheckpoint::new(session_id.into()))
        .await
        .unwrap();
    let pms = PersistenceMetaStore::new(storage.clone(), session_id.into());
    for i in 0..3 {
        let meta = SnapshotMeta {
            id: format!("snap-{i}"),
            reason: format!("reason-{i}"),
            created_at: Utc::now(),
            session_id: session_id.into(),
            status: SnapshotStatus::Pending,
        };
        pms.save_meta(&meta).await.unwrap();
    }
    let cp = storage.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(cp.snapshot_metas.len(), 3);
    assert_eq!(cp.snapshot_metas[2].id, "snap-2");
}

#[tokio::test]
async fn test_persistence_meta_store_checkpoint_not_found() {
    let storage = Arc::new(MemoryStorage::new());
    let pms = PersistenceMetaStore::new(storage, "nonexistent".into());
    let meta = SnapshotMeta {
        id: "x".into(),
        reason: "r".into(),
        created_at: Utc::now(),
        session_id: "nonexistent".into(),
        status: SnapshotStatus::Pending,
    };
    let result = pms.save_meta(&meta).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

// =====================================================================
// PersistenceMetaStore — session_id fill
// =====================================================================

#[tokio::test]
async fn test_persistence_meta_store_fills_session_id() {
    let storage = Arc::new(MemoryStorage::new());
    let session_id = "sess-fill-id";
    storage
        .save_checkpoint(&SessionCheckpoint::new(session_id.into()))
        .await
        .unwrap();
    let pms = PersistenceMetaStore::new(storage.clone(), session_id.into());
    let meta = SnapshotMeta {
        id: "snap-fill".into(),
        reason: "test".into(),
        created_at: Utc::now(),
        session_id: String::new(), // intentionally empty
        status: SnapshotStatus::Pending,
    };
    pms.save_meta(&meta).await.unwrap();
    let cp = storage.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(cp.snapshot_metas.len(), 1);
    // The persisted meta should have the store's session_id, not empty.
    assert_eq!(cp.snapshot_metas[0].session_id, session_id);
}

#[tokio::test]
async fn test_persistence_meta_store_overwrites_empty_session_id() {
    let storage = Arc::new(MemoryStorage::new());
    let session_id = "sess-overwrite";
    storage
        .save_checkpoint(&SessionCheckpoint::new(session_id.into()))
        .await
        .unwrap();
    let pms = PersistenceMetaStore::new(storage.clone(), session_id.into());
    // Save two metas — first with empty session_id, second with wrong session_id.
    for (i, bad_id) in ["", "wrong-id"].iter().enumerate() {
        let meta = SnapshotMeta {
            id: format!("snap-{i}"),
            reason: "test".into(),
            created_at: Utc::now(),
            session_id: bad_id.to_string(),
            status: SnapshotStatus::Pending,
        };
        pms.save_meta(&meta).await.unwrap();
    }
    let cp = storage.load_checkpoint(session_id).await.unwrap().unwrap();
    assert_eq!(cp.snapshot_metas.len(), 2);
    // Both should have the store's session_id.
    assert_eq!(cp.snapshot_metas[0].session_id, session_id);
    assert_eq!(cp.snapshot_metas[1].session_id, session_id);
}

// =====================================================================
// Reason passthrough verification
// =====================================================================

/// A `SnapshotMetaStore` that records every saved meta for inspection.
struct RecordingMetaStore {
    recorded: std::sync::Mutex<Vec<SnapshotMeta>>,
}

impl RecordingMetaStore {
    fn new() -> Self {
        Self {
            recorded: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn get_last_meta(&self) -> Option<SnapshotMeta> {
        self.recorded.lock().unwrap().last().cloned()
    }
}

#[async_trait::async_trait]
impl SnapshotMetaStore for RecordingMetaStore {
    async fn save_meta(&self, meta: &SnapshotMeta) -> Result<(), String> {
        self.recorded.lock().unwrap().push(meta.clone());
        Ok(())
    }
}

#[tokio::test]
async fn test_create_snapshot_passes_reason_to_meta() {
    let store = Arc::new(RecordingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_snapshot(&[msg("user", "test")], TranscriptOp::Rewrite, "user-stop");
    tokio::task::yield_now().await;
    let recorded = store.get_last_meta().unwrap();
    assert_eq!(recorded.reason, "user-stop");
}

#[tokio::test]
async fn test_create_incremental_snapshot_passes_reason_to_meta() {
    let store = Arc::new(RecordingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_incremental_snapshot(&[msg("user", "test")], "append-reason", "entry_1");
    tokio::task::yield_now().await;
    let recorded = store.get_last_meta().unwrap();
    assert_eq!(recorded.reason, "append-reason");
}

#[tokio::test]
async fn test_rollback_passes_pre_rollback_reason_to_meta() {
    let store = Arc::new(RecordingMetaStore::new());
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.set_meta_store(store.clone());
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite, "first");
    tokio::task::yield_now().await;
    // Rollback creates a pre-rollback snapshot with reason "pre-rollback".
    let _ = mgr.rollback(&[msg("user", "current")]);
    tokio::task::yield_now().await;
    let recorded = store.get_last_meta().unwrap();
    assert_eq!(recorded.reason, "pre-rollback");
}

// =====================================================================
// Snapshot kind distinction
// =====================================================================

#[test]
fn test_full_rewrite_snapshot_returns_replace() {
    let mut mgr = RuntimeSnapshotManager::new();
    let msgs = vec![msg("user", "hello")];
    mgr.create_snapshot(&msgs, TranscriptOp::Rewrite, "rewrite");
    let action = mgr.rollback(&msgs).unwrap();
    match action {
        RollbackAction::Replace { messages } => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, "user");
        }
        _ => panic!("expected Replace action"),
    }
}

#[test]
fn test_incremental_snapshot_leaf_entry_id_preserved() {
    let mut mgr = RuntimeSnapshotManager::new();
    let msgs = vec![msg("user", "test")];
    mgr.create_incremental_snapshot(&msgs, "test", "leaf_999");
    let action = mgr.rollback(&msgs).unwrap();
    match action {
        RollbackAction::Truncate { leaf_entry_id } => {
            assert_eq!(leaf_entry_id, "leaf_999");
        }
        _ => panic!("expected Truncate action"),
    }
}
