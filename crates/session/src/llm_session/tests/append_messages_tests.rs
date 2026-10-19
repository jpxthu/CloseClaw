use super::*;
use closeclaw_common::{ContentBlock, UnifiedResponse};

// ── SessionMessage serde roundtrip ────────────────────────────────────────

#[test]
fn test_session_message_serde_roundtrip() {
    let msg = SessionMessage {
        role: "user".into(),
        content_blocks: vec![
            ContentBlock::Text("hello".into()),
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "get_weather".into(),
                input: r#"{"city":"Tokyo"}"#.into(),
            },
        ],
        timestamp: Utc::now(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SessionMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(msg.role, parsed.role);
    assert_eq!(msg.content_blocks, parsed.content_blocks);
}

// ── ConversationSession initial state ─────────────────────────────────────

#[test]
fn test_conversation_session_new() {
    let session = ConversationSession::new("sess_42".into(), "gpt-4o".into(), tmp_path());
    assert_eq!(session.messages().len(), 0);
    assert_eq!(session.turn_count(), 0);
    assert!(session.system_prompt().is_none());
}

// ── append_response adds a message ─────────────────────────────────────────

#[test]
fn test_append_response_adds_message() {
    let mut session = ConversationSession::new("sess_1".into(), "gpt-4o".into(), tmp_path());
    let response = UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Hi there!".into())],
        usage: make_usage_basic(1, 2, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    };
    session.append_response(response);
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.messages()[0].role, "assistant");
}

// ── append_tool_result increments turn ────────────────────────────────────

#[test]
fn test_append_tool_result_increments_turn() {
    let mut session = ConversationSession::new("sess_2".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Using tool...".into())],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    assert_eq!(session.turn_count(), 0);
    session.append_tool_result("call_x".into(), "tool output".into());
    assert_eq!(session.turn_count(), 1);
}

// ── append_response with empty blocks does NOT increment turn ──────────────

#[test]
fn test_append_response_empty_blocks_no_turn_increment() {
    let mut session = ConversationSession::new("sess_3".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![],
        usage: make_usage_basic(0, 0, None),
        finish_reason: None,
        retry_attempts: 0,
    });
    assert_eq!(session.messages().len(), 0);
    assert_eq!(session.turn_count(), 0);
}

// ── Multiple turns ──────────────────────────────────────────────────────────

#[test]
fn test_conversation_session_multiple_turns() {
    let mut session = ConversationSession::new("sess_6".into(), "gpt-4o".into(), tmp_path());

    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("First response".into())],
        usage: make_usage_basic(1, 2, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.turn_count(), 0);

    session.append_tool_result("call_1".into(), "result A".into());
    assert_eq!(session.turn_count(), 1);

    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Second response".into())],
        usage: make_usage_basic(1, 3, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.turn_count(), 1);

    session.append_tool_result("call_2".into(), "result B".into());
    assert_eq!(session.turn_count(), 2);
    assert_eq!(session.messages().len(), 2);
}

// ── append_user_message persists user input ──────────────────────────────

#[test]
fn test_append_user_message_adds_user_role() {
    let mut session = ConversationSession::new("s_um1".into(), "gpt-4o".into(), tmp_path());
    session.append_user_message("hello world");
    assert_eq!(session.messages().len(), 1);
    assert_eq!(session.messages()[0].role, "user");
    assert_eq!(
        session.messages()[0].content_blocks,
        vec![ContentBlock::Text("hello world".into())]
    );
}

#[test]
fn test_append_user_message_preserves_order_with_assistant() {
    let mut session = ConversationSession::new("s_um2".into(), "gpt-4o".into(), tmp_path());
    session.append_user_message("question");
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("answer".into())],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[0].role, "user");
    assert_eq!(session.messages()[1].role, "assistant");
}

#[test]
fn test_append_user_message_compact_includes_user() {
    let mut session = ConversationSession::new("s_um3".into(), "gpt-4o".into(), tmp_path());
    session.append_user_message("what is rust?");
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Rust is a systems language.".into())],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    let req = session.build_api_request();
    // Should contain user + assistant messages in the API request
    assert!(req.messages.iter().any(|m| m.role == "user"));
    assert!(req.messages.iter().any(|m| m.role == "assistant"));
}

// ── model() getter ───────────────────────────────────────────────────────

#[test]
fn test_model_returns_model_name() {
    let session = ConversationSession::new("s1".into(), "glm-5.1".into(), tmp_path());
    assert_eq!(session.model(), "glm-5.1");
}

#[test]
fn test_model_returns_empty_string() {
    let session = ConversationSession::new("s2".into(), String::new(), tmp_path());
    assert_eq!(session.model(), "");
}
