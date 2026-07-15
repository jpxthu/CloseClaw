//! Unit tests for transcript operation methods.

use super::{ConversationSession, SessionMessage};
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

// ── append_transcript_with_snapshot ──────────────────────────────────────

use crate::run_health::RollbackAction;

#[test]
fn test_append_transcript_with_snapshot_creates_snapshot() {
    let mut cs = make_session("s5");
    cs.append_transcript("user", vec![ContentBlock::Text("init".into())]);
    assert_eq!(cs.snapshot_count(), None);
    let created = cs.append_transcript_with_snapshot(
        "assistant",
        vec![ContentBlock::Text("reply".into())],
        "entry_10",
    );
    assert!(created.is_some());
    assert_eq!(cs.snapshot_count(), Some(1));
    // The appended message is present.
    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.messages[1].role, "assistant");
}

#[test]
fn test_append_transcript_with_snapshot_rollback_returns_truncate() {
    let mut cs = make_session("s6");
    cs.append_transcript("user", vec![ContentBlock::Text("before".into())]);
    cs.append_transcript_with_snapshot(
        "assistant",
        vec![ContentBlock::Text("after".into())],
        "entry_42",
    );
    let action = cs.rollback_transcript().unwrap();
    match action {
        RollbackAction::Truncate { leaf_entry_id } => {
            assert_eq!(leaf_entry_id, "entry_42");
        }
        _ => panic!("expected Truncate action for incremental snapshot"),
    }
}

#[test]
fn test_append_transcript_with_snapshot_full_path() {
    let mut cs = make_session("s7");
    // Initial message.
    cs.append_transcript("user", vec![ContentBlock::Text("q1".into())]);
    // Append with snapshot — creates incremental snapshot of state before append.
    let created = cs.append_transcript_with_snapshot(
        "assistant",
        vec![ContentBlock::Text("a1".into())],
        "entry_99",
    );
    assert!(created.is_some());
    assert_eq!(cs.messages.len(), 2);
    // Rollback returns Truncate with the correct leaf_entry_id.
    let action = cs.rollback_transcript().unwrap();
    match action {
        RollbackAction::Truncate { leaf_entry_id } => {
            assert_eq!(leaf_entry_id, "entry_99");
        }
        _ => panic!("expected Truncate"),
    }
    // After rollback, the snapshot count reflects the pre-rollback sentinel.
    assert!(cs.snapshot_count().unwrap() >= 1);
}

// ── Convenience methods: verify they go through append_transcript ─────────

/// `append_user_message` adds a user message via `append_transcript`.
#[test]
fn test_append_user_message_via_append_transcript() {
    let mut cs = make_session("s8");
    cs.append_user_message("hello world");
    assert_eq!(cs.messages.len(), 1);
    assert_eq!(cs.messages[0].role, "user");
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::Text("hello world".into())
    );
}

/// `inject_system_message` adds a system message via `append_transcript`.
#[test]
fn test_inject_system_message_via_append_transcript() {
    let mut cs = make_session("s9");
    cs.inject_system_message("retry instruction".to_string());
    assert_eq!(cs.messages.len(), 1);
    assert_eq!(cs.messages[0].role, "system");
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::Text("retry instruction".into())
    );
}

/// `inject_tool_result` adds a tool result via `append_transcript`.
#[test]
fn test_inject_tool_result_via_append_transcript() {
    let mut cs = make_session("s10");
    cs.inject_tool_result("call_1", "tool output");
    assert_eq!(cs.messages.len(), 1);
    assert_eq!(cs.messages[0].role, "tool");
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::ToolResult {
            tool_call_id: "call_1".into(),
            content: "tool output".into(),
        }
    );
}

/// `clone_messages_from` appends multiple messages preserving timestamps.
#[test]
fn test_clone_messages_from_via_append_transcript() {
    use chrono::Utc;
    let source = vec![
        SessionMessage {
            role: "user".into(),
            content_blocks: vec![ContentBlock::Text("q".into())],
            timestamp: Utc::now(),
        },
        SessionMessage {
            role: "assistant".into(),
            content_blocks: vec![ContentBlock::Text("a".into())],
            timestamp: Utc::now(),
        },
    ];
    let mut cs = make_session("s11");
    cs.clone_messages_from(&source);
    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.messages[0].role, "user");
    assert_eq!(cs.messages[1].role, "assistant");
}

/// Convenience methods do not create snapshots (Append does not require snapshot).
#[test]
fn test_convenience_methods_no_snapshot() {
    let mut cs = make_session("s12");
    cs.append_user_message("msg1");
    cs.inject_system_message("sys".to_string());
    cs.inject_tool_result("t1", "res");
    // Append operations should not create snapshots.
    assert_eq!(cs.snapshot_count(), None);
}
