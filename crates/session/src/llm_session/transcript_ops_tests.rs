//! Unit tests for transcript operation methods.

use super::ConversationSession;
use crate::run_health::TranscriptOp;
use closeclaw_common::ContentBlock;
use std::path::PathBuf;

fn make_session(id: &str) -> ConversationSession {
    ConversationSession::new(id.into(), "test-model".into(), PathBuf::from("/tmp"))
}

// ── snapshot_current_state ────────────────────────────────────────────────

#[test]
fn test_snapshot_current_state_rewrite_creates_snapshot() {
    let mut cs = make_session("s1");
    cs.append_transcript("user", vec![ContentBlock::Text("hello".into())]);
    assert_eq!(cs.snapshot_count(), None);
    cs.snapshot_current_state(TranscriptOp::Rewrite, "test");
    assert_eq!(cs.snapshot_count(), Some(1));
}

#[test]
fn test_snapshot_current_state_partial_rewrite_creates_snapshot() {
    let mut cs = make_session("s2");
    cs.append_transcript("system", vec![ContentBlock::Text("prompt".into())]);
    cs.snapshot_current_state(TranscriptOp::PartialRewrite, "test");
    assert_eq!(cs.snapshot_count(), Some(1));
}

#[test]
fn test_snapshot_current_state_append_no_snapshot() {
    let mut cs = make_session("s3");
    cs.append_transcript("user", vec![ContentBlock::Text("msg".into())]);
    cs.snapshot_current_state(TranscriptOp::Append, "test");
    assert_eq!(cs.snapshot_count(), None);
}

#[test]
fn test_snapshot_current_state_is_undoable() {
    let mut cs = make_session("s4");
    cs.append_transcript("user", vec![ContentBlock::Text("before".into())]);
    cs.snapshot_current_state(TranscriptOp::Rewrite, "test");
    // Rollback should restore the "before" state.
    let action = cs.rollback_transcript();
    assert!(action.is_some());
    // Messages should be restored to the snapshot state.
    assert_eq!(cs.messages.len(), 1);
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::Text("before".into())
    );
}
