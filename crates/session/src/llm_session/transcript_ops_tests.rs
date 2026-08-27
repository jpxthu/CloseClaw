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

// ── truncate_transcript_to_limit ─────────────────────────────────────────

/// Normal path: history > max → oldest messages removed, most recent
/// `max` messages retained; returns the number of dropped messages.
#[test]
fn test_truncate_normal_path_drops_oldest() {
    let mut cs = make_session("t1");
    // Append 5 messages: m0..m4
    for i in 0..5 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    assert_eq!(cs.messages.len(), 5);

    let dropped = cs.truncate_transcript_to_limit(Some(3));

    assert_eq!(dropped, 2, "should drop 2 oldest messages");
    assert_eq!(cs.messages.len(), 3, "should retain 3 messages");
    // Oldest two (msg0, msg1) are gone; newest three remain.
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::Text("msg2".into())
    );
    assert_eq!(
        cs.messages[1].content_blocks[0],
        ContentBlock::Text("msg3".into())
    );
    assert_eq!(
        cs.messages[2].content_blocks[0],
        ContentBlock::Text("msg4".into())
    );
}

/// Boundary: message count == max → no truncation, returns 0.
#[test]
fn test_truncate_at_limit_no_op() {
    let mut cs = make_session("t2");
    for i in 0..3 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let dropped = cs.truncate_transcript_to_limit(Some(3));
    assert_eq!(dropped, 0, "no messages should be dropped at the limit");
    assert_eq!(cs.messages.len(), 3);
    assert_eq!(cs.snapshot_count(), None, "no snapshot should be created");
}

/// Boundary: message count < max → no truncation, returns 0.
#[test]
fn test_truncate_below_limit_no_op() {
    let mut cs = make_session("t3");
    for i in 0..2 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let dropped = cs.truncate_transcript_to_limit(Some(5));
    assert_eq!(dropped, 0);
    assert_eq!(cs.messages.len(), 2);
    assert_eq!(cs.snapshot_count(), None);
}

/// Boundary: max = None → no truncation, returns 0.
#[test]
fn test_truncate_none_max_no_op() {
    let mut cs = make_session("t4");
    for i in 0..10 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let dropped = cs.truncate_transcript_to_limit(None);
    assert_eq!(dropped, 0);
    assert_eq!(cs.messages.len(), 10);
    assert_eq!(cs.snapshot_count(), None);
}

/// Snapshot: truncation creates a PartialRewrite snapshot that can be
/// rolled back to restore the pre-truncation state.
#[test]
fn test_truncate_creates_undoable_snapshot() {
    let mut cs = make_session("t5");
    for i in 0..5 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let dropped = cs.truncate_transcript_to_limit(Some(3));
    assert_eq!(dropped, 2);
    assert_eq!(cs.messages.len(), 3);
    // A snapshot should exist after truncation.
    assert_eq!(cs.snapshot_count(), Some(1));

    // Rollback restores the pre-truncation state (5 messages).
    let action = cs.rollback_transcript();
    assert!(action.is_some(), "rollback should succeed");
    match action.unwrap() {
        RollbackAction::Replace { messages } => {
            assert_eq!(messages.len(), 5, "rollback should restore all 5 messages");
        }
        _ => panic!("expected Replace action for PartialRewrite snapshot"),
    }
}

/// last_activity_at is updated after a truncation that actually drops messages.
#[test]
fn test_truncate_updates_last_activity_at() {
    let mut cs = make_session("t6");
    cs.append_transcript("user", vec![ContentBlock::Text("a".into())]);
    cs.append_transcript("user", vec![ContentBlock::Text("b".into())]);
    cs.append_transcript("user", vec![ContentBlock::Text("c".into())]);
    let before = cs.last_activity_at();
    // Small sleep to ensure timestamp difference (1-second resolution).
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let _ = cs.truncate_transcript_to_limit(Some(2));
    let after = cs.last_activity_at();
    assert!(
        after > before,
        "last_activity_at should be updated after truncation"
    );
}

/// Truncation with mixed roles preserves the most recent messages
/// regardless of role — the transcript is treated as an ordered log.
#[test]
fn test_truncate_preserves_newest_mixed_roles() {
    let mut cs = make_session("t7");
    cs.append_transcript("system", vec![ContentBlock::Text("sys".into())]);
    cs.append_transcript("user", vec![ContentBlock::Text("q1".into())]);
    cs.append_transcript("assistant", vec![ContentBlock::Text("a1".into())]);
    cs.append_transcript("user", vec![ContentBlock::Text("q2".into())]);
    cs.append_transcript("assistant", vec![ContentBlock::Text("a2".into())]);
    // 5 messages, keep 3.
    let dropped = cs.truncate_transcript_to_limit(Some(3));
    assert_eq!(dropped, 2);
    assert_eq!(cs.messages.len(), 3);
    assert_eq!(cs.messages[0].role, "assistant"); // a1
    assert_eq!(cs.messages[1].role, "user"); // q2
    assert_eq!(cs.messages[2].role, "assistant"); // a2
}

/// Truncation on empty session returns 0 and leaves messages empty.
#[test]
fn test_truncate_empty_session_no_op() {
    let mut cs = make_session("t8");
    let dropped = cs.truncate_transcript_to_limit(Some(5));
    assert_eq!(dropped, 0);
    assert_eq!(cs.messages.len(), 0);
    assert_eq!(cs.snapshot_count(), None);
}

/// Two consecutive truncations: the second operates on already-truncated
/// history and only one snapshot is created per truncation.
#[test]
fn test_truncate_consecutive_truncations() {
    let mut cs = make_session("t9");
    for i in 0..10 {
        cs.append_transcript("user", vec![ContentBlock::Text(format!("msg{i}"))]);
    }
    let d1 = cs.truncate_transcript_to_limit(Some(5));
    assert_eq!(d1, 5);
    assert_eq!(cs.messages.len(), 5);
    assert_eq!(cs.snapshot_count(), Some(1));

    let d2 = cs.truncate_transcript_to_limit(Some(3));
    assert_eq!(d2, 2);
    assert_eq!(cs.messages.len(), 3);
    assert_eq!(cs.snapshot_count(), Some(2));
    // Most recent 3 messages: msg7, msg8, msg9.
    assert_eq!(
        cs.messages[0].content_blocks[0],
        ContentBlock::Text("msg7".into())
    );
    assert_eq!(
        cs.messages[2].content_blocks[0],
        ContentBlock::Text("msg9".into())
    );
}
