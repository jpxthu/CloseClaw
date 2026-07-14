//! Tests for `ConversationSession::recent_tool_calls`.

use super::*;
use closeclaw_common::{ContentBlock, UnifiedResponse, UnifiedUsage};

fn usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: Some(0),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn assistant_with_tools(id: &str, tool_name: &str, input: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![
            ContentBlock::Text(format!("calling {tool_name}")),
            ContentBlock::ToolUse {
                id: id.into(),
                name: tool_name.into(),
                input: input.into(),
            },
        ],
        usage: usage(),
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    }
}

fn assistant_text(text: &str) -> UnifiedResponse {
    UnifiedResponse {
        content_blocks: vec![ContentBlock::Text(text.into())],
        usage: usage(),
        finish_reason: Some("stop".into()),
        retry_attempts: 0,
    }
}

#[test]
fn test_empty_session_returns_empty() {
    let session = ConversationSession::new("s1".into(), "m".into(), tmp_path());
    assert!(session.recent_tool_calls(5).is_empty());
}

#[test]
fn test_returns_only_tool_use_blocks() {
    let mut session = ConversationSession::new("s2".into(), "m".into(), tmp_path());
    session.append_response(assistant_with_tools("c1", "read", r#"{"path":"/a"}"#));
    // ToolUse + ToolResult in same message → 1 call
    session.append_tool_result("c1".into(), "content".into());

    let calls = session.recent_tool_calls(5);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].input, r#"{"path":"/a"}"#);
}

#[test]
fn test_returns_multiple_tools_from_single_turn() {
    let mut session = ConversationSession::new("s3".into(), "m".into(), tmp_path());
    session.append_response(UnifiedResponse {
        content_blocks: vec![
            ContentBlock::ToolUse {
                id: "a".into(),
                name: "read".into(),
                input: "{}".into(),
            },
            ContentBlock::ToolUse {
                id: "b".into(),
                name: "write".into(),
                input: "{}".into(),
            },
        ],
        usage: usage(),
        finish_reason: Some("tool_calls".into()),
        retry_attempts: 0,
    });

    let calls = session.recent_tool_calls(5);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[1].name, "write");
}

#[test]
fn test_limits_to_n_messages() {
    let mut session = ConversationSession::new("s4".into(), "m".into(), tmp_path());
    // 3 assistant messages with tool calls
    for i in 0..3 {
        session.append_response(assistant_with_tools(
            &format!("c{i}"),
            "exec",
            &format!("cmd{i}"),
        ));
    }

    // take(2) includes c2 and c3 → 2 tool calls
    let calls_2 = session.recent_tool_calls(2);
    assert_eq!(calls_2.len(), 2);
    assert_eq!(calls_2[0].input, "cmd1");
    assert_eq!(calls_2[1].input, "cmd2");

    let calls_all = session.recent_tool_calls(10);
    assert_eq!(calls_all.len(), 3);
}

#[test]
fn test_tool_result_blocks_are_excluded() {
    let mut session = ConversationSession::new("s5".into(), "m".into(), tmp_path());
    session.append_response(assistant_with_tools("c1", "exec", "ls"));
    session.append_tool_result("c1".into(), "file.txt".into());

    let calls = session.recent_tool_calls(5);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "exec");
}

#[test]
fn test_chronological_order_preserved() {
    let mut session = ConversationSession::new("s6".into(), "m".into(), tmp_path());
    session.append_response(assistant_with_tools("c1", "a", "1"));
    session.append_response(assistant_with_tools("c2", "b", "2"));
    session.append_response(assistant_with_tools("c3", "c", "3"));

    let calls = session.recent_tool_calls(3);
    assert_eq!(calls[0].name, "a");
    assert_eq!(calls[1].name, "b");
    assert_eq!(calls[2].name, "c");
}

#[test]
fn test_n_zero_returns_empty() {
    let mut session = ConversationSession::new("s7".into(), "m".into(), tmp_path());
    session.append_response(assistant_with_tools("c1", "read", "{}"));
    assert!(session.recent_tool_calls(0).is_empty());
}

#[test]
fn test_skips_text_only_messages() {
    let mut session = ConversationSession::new("s8".into(), "m".into(), tmp_path());
    session.append_response(assistant_text("just text"));
    session.append_response(assistant_with_tools("c1", "read", "{}"));
    session.append_response(assistant_text("more text"));

    let calls = session.recent_tool_calls(3);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read");
}

#[test]
fn test_n_one_returns_last_message_tools() {
    let mut session = ConversationSession::new("s9".into(), "m".into(), tmp_path());
    session.append_response(assistant_with_tools("c1", "old_tool", "{}"));
    session.append_response(assistant_with_tools("c2", "new_tool", "{}"));

    let calls = session.recent_tool_calls(1);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "new_tool");
}
