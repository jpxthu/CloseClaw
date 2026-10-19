use super::*;
use closeclaw_common::{ContentBlock, UnifiedResponse};

// ── build_api_request with system_prompt ───────────────────────────────────

#[test]
fn test_build_api_request_includes_system_prompt() {
    let session = ConversationSession::new("sess_4".into(), "gpt-4o".into(), tmp_path())
        .with_system_prompt("You are helpful.");
    let req = session.build_api_request();
    assert!(req
        .messages
        .iter()
        .any(|m| m.role == "system" && m.content.contains("helpful")));
}

// ── build_api_request without system_prompt ────────────────────────────────

#[test]
fn test_build_api_request_without_system_prompt() {
    let mut session = ConversationSession::new("sess_5".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Who are you?".into())],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    let req = session.build_api_request();
    assert!(!req.messages.is_empty());
    assert!(!req.messages.iter().any(|m| m.role == "system"));
}

// ── build_api_request ToolResult separation ──────────────────────────────

#[test]
fn test_build_api_request_separates_tool_result_messages() {
    let mut session = ConversationSession::new("sess_tool_sep".into(), "gpt-4o".into(), tmp_path());
    // Assistant response with text + tool use
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text("Let me check.".into()),
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "search".into(),
                input: "query".into(),
            },
        ],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    });
    // Append tool result
    session.append_tool_result("call_1".into(), "result: 42".into());

    let req = session.build_api_request();
    // Should have: system (none), assistant, tool = 2 messages
    assert_eq!(req.messages.len(), 2);

    // First message: assistant with non-tool blocks only
    let assistant_msg = &req.messages[0];
    assert_eq!(assistant_msg.role, "assistant");
    assert!(!assistant_msg.content.is_empty());
    assert!(assistant_msg.tool_call_id.is_none());

    // Second message: independent tool result
    let tool_msg = &req.messages[1];
    assert_eq!(tool_msg.role, "tool");
    assert_eq!(tool_msg.content, "result: 42");
    assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn test_build_api_request_multiple_tool_results_appended_in_order() {
    let mut session =
        ConversationSession::new("sess_tool_multi".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text("Thinking...".into()),
            ContentBlock::ToolUse {
                id: "call_a".into(),
                name: "tool_a".into(),
                input: "{}".into(),
            },
            ContentBlock::ToolUse {
                id: "call_b".into(),
                name: "tool_b".into(),
                input: "{}".into(),
            },
        ],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    });
    session.append_tool_result("call_a".into(), "result_a".into());
    session.append_tool_result("call_b".into(), "result_b".into());

    let req = session.build_api_request();
    // assistant + 2 tool results = 3 messages
    assert_eq!(req.messages.len(), 3);
    assert_eq!(req.messages[0].role, "assistant");
    assert_eq!(req.messages[1].role, "tool");
    assert_eq!(req.messages[1].tool_call_id.as_deref(), Some("call_a"));
    assert_eq!(req.messages[1].content, "result_a");
    assert_eq!(req.messages[2].role, "tool");
    assert_eq!(req.messages[2].tool_call_id.as_deref(), Some("call_b"));
    assert_eq!(req.messages[2].content, "result_b");
}

#[test]
fn test_build_api_request_no_tool_result_preserves_original_behavior() {
    let mut session = ConversationSession::new("sess_no_tool".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("Hello".into())],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });

    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].role, "assistant");
    assert_eq!(req.messages[0].content, "Hello");
    assert!(req.messages[0].tool_call_id.is_none());
}

// ── build_api_request Audio / File flatten tokens ────────────────────────

#[test]
fn test_build_api_request_audio_block_flattens_to_audio_token() {
    let mut session =
        ConversationSession::new("sess_audio_tok".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::Audio {
            name: "clip.mp3".into(),
            url: "data:audio/mpeg;base64,AAA".into(),
        }],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].content, "[audio: clip.mp3]");
    assert!(
        req.messages[0].content_blocks.is_none(),
        "Audio blocks flatten to text; only Image blocks are kept structured"
    );
}

#[test]
fn test_build_api_request_file_block_flattens_to_file_token() {
    let mut session = ConversationSession::new("sess_file_tok".into(), "gpt-4o".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![ContentBlock::File {
            name: "report.pdf".into(),
            url: "data:application/pdf;base64,AAA".into(),
        }],
        usage: make_usage_basic(1, 1, None),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    });
    let req = session.build_api_request();
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.messages[0].content, "[file: report.pdf]");
    assert!(req.messages[0].content_blocks.is_none());
}
