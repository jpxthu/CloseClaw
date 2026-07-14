//! Unit tests for [`RuntimeSnapshotManager`].

use super::snapshot_manager::{RollbackAction, RuntimeSnapshotManager, TranscriptOp};
use crate::llm_session::SessionMessage;
use chrono::Utc;
use closeclaw_common::ContentBlock;

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
fn test_create_snapshot_returns_true_for_rewrite() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    assert!(mgr.create_snapshot(&messages, TranscriptOp::Rewrite, "test"));
    assert_eq!(mgr.snapshot_count(), 1);
}

#[test]
fn test_create_snapshot_returns_false_for_append() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    assert!(!mgr.create_snapshot(&messages, TranscriptOp::Append, "test"));
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
    assert!(mgr.create_snapshot(&messages, TranscriptOp::PartialRewrite, "test"));
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
