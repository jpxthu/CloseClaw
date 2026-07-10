//! Unit tests for [`RuntimeSnapshotManager`].

use super::snapshot_manager::{RuntimeSnapshotManager, TranscriptOp};
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
    assert!(mgr.create_snapshot(&messages, TranscriptOp::Rewrite));
    assert_eq!(mgr.snapshot_count(), 1);
}

#[test]
fn test_create_snapshot_returns_false_for_append() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "hello")];
    assert!(!mgr.create_snapshot(&messages, TranscriptOp::Append));
    assert_eq!(mgr.snapshot_count(), 0);
}

#[test]
fn test_create_snapshot_stores_messages() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("user", "a"), msg("assistant", "b")];
    mgr.create_snapshot(&messages, TranscriptOp::Rewrite);

    let restored = mgr.rollback().unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].role, "user");
    assert_eq!(restored[1].role, "assistant");
}

#[test]
fn test_create_snapshot_partial_rewrite() {
    let mut mgr = RuntimeSnapshotManager::new();
    let messages = vec![msg("system", "system prompt")];
    assert!(mgr.create_snapshot(&messages, TranscriptOp::PartialRewrite));
    assert_eq!(mgr.snapshot_count(), 1);
}

// =====================================================================
// rollback
// =====================================================================

#[test]
fn test_rollback_returns_most_recent() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "first")], TranscriptOp::Rewrite);
    mgr.create_snapshot(
        &[msg("user", "first"), msg("assistant", "second")],
        TranscriptOp::Rewrite,
    );

    let restored = mgr.rollback().unwrap();
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[1].role, "assistant");

    // After rolling back the most recent, the earlier one remains.
    assert_eq!(mgr.snapshot_count(), 1);
    let restored2 = mgr.rollback().unwrap();
    assert_eq!(restored2.len(), 1);
}

#[test]
fn test_rollback_returns_none_when_empty() {
    let mut mgr = RuntimeSnapshotManager::new();
    assert!(mgr.rollback().is_none());
}

#[test]
fn test_rollback_clears_snapshot() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "data")], TranscriptOp::Rewrite);
    assert_eq!(mgr.snapshot_count(), 1);
    mgr.rollback();
    assert_eq!(mgr.snapshot_count(), 0);
}

// =====================================================================
// 25-snapshot limit
// =====================================================================

#[test]
fn test_max_snapshots_evicts_oldest() {
    let mut mgr = RuntimeSnapshotManager::new();
    for i in 0..30 {
        mgr.create_snapshot(&[msg("user", &format!("msg-{i}"))], TranscriptOp::Rewrite);
    }
    assert_eq!(mgr.snapshot_count(), 25);

    // The oldest 5 should have been evicted.
    // The remaining snapshots should be msg-5 through msg-29.
    // Rolling back pops from the back (msg-29).
    let restored = mgr.rollback().unwrap();
    assert_eq!(
        restored[0].content_blocks[0],
        ContentBlock::Text("msg-29".into())
    );
}

#[test]
fn test_exactly_at_limit_no_eviction() {
    let mut mgr = RuntimeSnapshotManager::new();
    for i in 0..25 {
        mgr.create_snapshot(&[msg("user", &format!("msg-{i}"))], TranscriptOp::Rewrite);
    }
    assert_eq!(mgr.snapshot_count(), 25);

    // Add one more — oldest (msg-0) should be evicted.
    mgr.create_snapshot(&[msg("user", "msg-25")], TranscriptOp::Rewrite);
    assert_eq!(mgr.snapshot_count(), 25);

    // Rollback the newest.
    let restored = mgr.rollback().unwrap();
    assert_eq!(
        restored[0].content_blocks[0],
        ContentBlock::Text("msg-25".into())
    );
}

// =====================================================================
// clear
// =====================================================================

#[test]
fn test_clear_removes_all_snapshots() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "a")], TranscriptOp::Rewrite);
    mgr.create_snapshot(&[msg("user", "b")], TranscriptOp::Rewrite);
    assert_eq!(mgr.snapshot_count(), 2);
    mgr.clear();
    assert_eq!(mgr.snapshot_count(), 0);
    assert!(mgr.rollback().is_none());
}

// =====================================================================
// Interleaved append + rewrite
// =====================================================================

#[test]
fn test_append_does_not_affect_snapshot_count() {
    let mut mgr = RuntimeSnapshotManager::new();
    mgr.create_snapshot(&[msg("user", "rewrite")], TranscriptOp::Rewrite);
    assert_eq!(mgr.snapshot_count(), 1);

    // Append operations should not create snapshots.
    mgr.create_snapshot(&[msg("user", "append1")], TranscriptOp::Append);
    mgr.create_snapshot(&[msg("user", "append2")], TranscriptOp::Append);
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
