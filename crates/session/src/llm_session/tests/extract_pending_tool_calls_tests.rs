//! Tests for `ConversationSession::extract_pending_tool_calls`.
//!
//! Covers Step 1.1 of the graceful-shutdown plan: scanning the last
//! assistant message for `ContentBlock::ToolUse` blocks and converting
//! them into `PendingOperation` entries.

use super::super::*;
use crate::persistence::PendingOperationType;
use crate::run_health::TranscriptOp;
use closeclaw_common::ContentBlock;

// ── helpers ──────────────────────────────────────────────────────────────

fn make_session(id: &str) -> ConversationSession {
    ConversationSession::new(id.to_string(), "gpt-4o".to_string(), tmp_path())
}

/// Set messages on the session via `apply_transcript_op` for full
/// control over message history.
fn set_messages(session: &mut ConversationSession, msgs: Vec<SessionMessage>) {
    session.apply_transcript_op(TranscriptOp::Rewrite, msgs);
}

fn assistant_msg(blocks: Vec<ContentBlock>) -> SessionMessage {
    SessionMessage {
        role: "assistant".to_string(),
        content_blocks: blocks,
        timestamp: Utc::now(),
    }
}

fn user_msg(text: &str) -> SessionMessage {
    SessionMessage {
        role: "user".to_string(),
        content_blocks: vec![ContentBlock::Text(text.to_string())],
        timestamp: Utc::now(),
    }
}

// ── Step 1.1: extract_pending_tool_calls ────────────────────────────────

#[test]
fn test_extract_pending_tool_calls_empty() {
    let session = make_session("extract_empty");
    let ops = session.extract_pending_tool_calls();
    assert!(ops.is_empty(), "no messages → empty result");
}

#[test]
fn test_extract_pending_tool_calls_no_tool_use() {
    let mut session = make_session("extract_no_tools");
    set_messages(
        &mut session,
        vec![
            user_msg("hello"),
            assistant_msg(vec![ContentBlock::Text("Hi there!".to_string())]),
        ],
    );
    let ops = session.extract_pending_tool_calls();
    assert!(ops.is_empty(), "text-only assistant message → empty result");
}

#[test]
fn test_extract_pending_tool_calls_with_tools() {
    let mut session = make_session("extract_with_tools");
    set_messages(
        &mut session,
        vec![
            user_msg("check the weather"),
            assistant_msg(vec![
                ContentBlock::Text("Let me check.".to_string()),
                ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    input: r#"{"city":"Tokyo"}"#.to_string(),
                },
                ContentBlock::ToolUse {
                    id: "call_2".to_string(),
                    name: "get_time".to_string(),
                    input: r#"{"tz":"Asia/Tokyo"}"#.to_string(),
                },
            ]),
        ],
    );
    let ops = session.extract_pending_tool_calls();
    assert_eq!(
        ops.len(),
        2,
        "should return one PendingOperation per ToolUse"
    );

    assert_eq!(ops[0].op_id, "call_1");
    assert!(matches!(ops[0].op_type, PendingOperationType::ToolCall));
    assert_eq!(ops[0].detail.tool_name(), Some("get_weather"));
    assert_eq!(ops[0].detail.args_summary(), Some(r#"{"city":"Tokyo"}"#));

    assert_eq!(ops[1].op_id, "call_2");
    assert!(matches!(ops[1].op_type, PendingOperationType::ToolCall));
    assert_eq!(ops[1].detail.tool_name(), Some("get_time"));
    assert_eq!(ops[1].detail.args_summary(), Some(r#"{"tz":"Asia/Tokyo"}"#));
}

#[test]
fn test_extract_pending_tool_calls_only_last_assistant() {
    let mut session = make_session("extract_only_last");
    set_messages(
        &mut session,
        vec![
            user_msg("first turn"),
            // First assistant message — has a ToolUse, but should be ignored.
            assistant_msg(vec![ContentBlock::ToolUse {
                id: "old_call".to_string(),
                name: "old_tool".to_string(),
                input: "{}".to_string(),
            }]),
            user_msg("second turn"),
            // Last assistant message — text only, no ToolUse.
            assistant_msg(vec![ContentBlock::Text("Done.".to_string())]),
        ],
    );
    let ops = session.extract_pending_tool_calls();
    assert!(
        ops.is_empty(),
        "must only inspect the LAST assistant message"
    );
}

#[test]
fn test_extract_pending_tool_calls_last_has_mixed_blocks() {
    let mut session = make_session("extract_mixed");
    set_messages(
        &mut session,
        vec![
            user_msg("do two things"),
            assistant_msg(vec![
                ContentBlock::Text("Thinking...".to_string()),
                ContentBlock::ToolUse {
                    id: "call_a".to_string(),
                    name: "tool_a".to_string(),
                    input: "a".to_string(),
                },
                ContentBlock::Thinking {
                    thinking: "internal note".to_string(),
                    signature: None,
                },
                ContentBlock::ToolUse {
                    id: "call_b".to_string(),
                    name: "tool_b".to_string(),
                    input: "b".to_string(),
                },
            ]),
        ],
    );
    let ops = session.extract_pending_tool_calls();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].op_id, "call_a");
    assert_eq!(ops[1].op_id, "call_b");
}

#[test]
fn test_extract_pending_tool_calls_does_not_modify_messages() {
    let mut session = make_session("extract_no_side_effect");
    let original_messages = vec![
        user_msg("hey"),
        assistant_msg(vec![ContentBlock::ToolUse {
            id: "call_x".to_string(),
            name: "tool_x".to_string(),
            input: "x".to_string(),
        }]),
    ];
    set_messages(&mut session, original_messages.clone());

    let _ops = session.extract_pending_tool_calls();

    // Messages must be unchanged.
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0].role, "user");
    assert_eq!(session.messages()[1].role, "assistant");
    assert_eq!(session.messages()[1].content_blocks.len(), 1);
    assert!(matches!(
        session.messages()[1].content_blocks[0],
        ContentBlock::ToolUse { .. }
    ));
}

#[test]
fn test_extract_pending_tool_calls_last_is_user_not_assistant() {
    // When the last message is a user message, extract_pending_tool_calls
    // still scans for the most recent assistant message (iterating in
    // reverse). If that assistant message contains ToolUse, those are
    // returned.
    let mut session = make_session("extract_last_user");
    set_messages(
        &mut session,
        vec![
            user_msg("first"),
            assistant_msg(vec![ContentBlock::ToolUse {
                id: "old_call".to_string(),
                name: "old".to_string(),
                input: "{}".to_string(),
            }]),
            user_msg("second"),
        ],
    );
    // The last assistant message (position 1) has ToolUse → returned.
    let ops = session.extract_pending_tool_calls();
    assert_eq!(ops.len(), 1, "should find ToolUse in last assistant msg");
    assert_eq!(ops[0].op_id, "old_call");
}

#[test]
fn test_extract_pending_tool_calls_last_assistant_empty_blocks() {
    let mut session = make_session("extract_empty_blocks");
    set_messages(&mut session, vec![user_msg("hi"), assistant_msg(vec![])]);
    let ops = session.extract_pending_tool_calls();
    assert!(ops.is_empty(), "empty content blocks → no ToolUse");
}
