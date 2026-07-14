//! Unit tests for `build_health_check_input` structural validation
//! and retry_attempts propagation.

use closeclaw_common::processor::{ContentBlock, UnifiedResponse, UnifiedUsage};

use super::health_check_builders::build_health_check_input;
use super::outbound::StreamResult;

fn make_usage() -> UnifiedUsage {
    UnifiedUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: Some(30),
        reasoning_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
    }
}

fn make_stream_result(blocks: Vec<ContentBlock>) -> StreamResult {
    StreamResult {
        content_blocks: blocks,
        usage: make_usage(),
        retry_attempts: 0,
    }
}

// ------------------------------------------------------------------
// Structural validation: normal / anomaly / mixed / empty
// ------------------------------------------------------------------

#[test]
fn test_normal_valid_blocks() {
    let result = make_stream_result(vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "search".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "t1".into(),
            content: "ok".into(),
        },
    ]);
    let input = build_health_check_input(&result, 500);
    assert!(input.is_structurally_valid);
    assert!(input.structural_anomaly_detail.is_none());
    assert!(input.has_text);
    assert!(input.has_tool_calls);
}

#[test]
fn test_tool_use_empty_id() {
    let result = make_stream_result(vec![ContentBlock::ToolUse {
        id: "".into(),
        name: "search".into(),
        input: "{}".into(),
    }]);
    let input = build_health_check_input(&result, 100);
    assert!(!input.is_structurally_valid);
    let detail = input.structural_anomaly_detail.expect("detail present");
    assert!(
        detail.contains("ToolUse"),
        "detail should mention ToolUse: {detail}"
    );
}

#[test]
fn test_tool_use_empty_name() {
    let result = make_stream_result(vec![ContentBlock::ToolUse {
        id: "t1".into(),
        name: "".into(),
        input: "{}".into(),
    }]);
    let input = build_health_check_input(&result, 100);
    assert!(!input.is_structurally_valid);
    let detail = input.structural_anomaly_detail.expect("detail present");
    assert!(
        detail.contains("ToolUse"),
        "detail should mention ToolUse: {detail}"
    );
}

#[test]
fn test_tool_result_empty_tool_call_id() {
    let result = make_stream_result(vec![ContentBlock::ToolResult {
        tool_call_id: "".into(),
        content: "result".into(),
    }]);
    let input = build_health_check_input(&result, 100);
    assert!(!input.is_structurally_valid);
    let detail = input.structural_anomaly_detail.expect("detail present");
    assert!(
        detail.contains("ToolResult"),
        "detail should mention ToolResult: {detail}"
    );
}

#[test]
fn test_mixed_valid_and_invalid_detects_first() {
    let result = make_stream_result(vec![
        ContentBlock::Text("ok".into()),
        ContentBlock::ToolUse {
            id: "".into(),
            name: "fetch".into(),
            input: "{}".into(),
        },
        ContentBlock::ToolResult {
            tool_call_id: "".into(),
            content: "fail".into(),
        },
    ]);
    let input = build_health_check_input(&result, 100);
    assert!(!input.is_structurally_valid);
    let detail = input.structural_anomaly_detail.expect("detail present");
    assert!(
        detail.contains("ToolUse"),
        "first invalid block is ToolUse: {detail}"
    );
}

#[test]
fn test_empty_content_blocks() {
    let result = make_stream_result(vec![]);
    let input = build_health_check_input(&result, 0);
    assert!(input.is_structurally_valid);
    assert!(input.structural_anomaly_detail.is_none());
    assert!(!input.has_text);
    assert!(!input.has_tool_calls);
}

// ------------------------------------------------------------------
// Retry attempts: deserialization default and From conversion
// ------------------------------------------------------------------

#[test]
fn test_unified_response_deserialize_default_retry_attempts() {
    let json = r#"{
        "content_blocks": [{"type": "text", "content": "hi"}],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 2,
            "total_tokens": 3
        }
    }"#;
    let resp: UnifiedResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.retry_attempts, 0);
}

#[test]
fn test_stream_result_from_unified_response_preserves_retry_attempts() {
    let resp = UnifiedResponse {
        content_blocks: vec![ContentBlock::Text("hello".into())],
        usage: make_usage(),
        finish_reason: Some("stop".into()),
        retry_attempts: 3,
    };
    let sr: StreamResult = resp.into();
    assert_eq!(sr.retry_attempts, 3);
}

// ------------------------------------------------------------------
// build_hook_context: recent_tool_calls parameter
// ------------------------------------------------------------------

use super::health_check_builders::build_hook_context;
use closeclaw_session::run_health::HookToolCallInfo;

#[test]
fn test_build_hook_context_no_recent_calls() {
    let result = make_stream_result(vec![
        ContentBlock::Text("hello".into()),
        ContentBlock::ToolUse {
            id: "t1".into(),
            name: "read".into(),
            input: r#"{"path":"/a"}"#.into(),
        },
    ]);
    let ctx = build_hook_context(&result, Vec::new());
    assert_eq!(ctx.text, "hello");
    assert_eq!(ctx.tool_calls.len(), 1);
    assert_eq!(ctx.tool_calls[0].name, "read");
    assert!(ctx.recent_tool_calls.is_empty());
}

#[test]
fn test_build_hook_context_with_recent_calls() {
    let result = make_stream_result(vec![ContentBlock::Text("done".into())]);
    let recent = vec![
        HookToolCallInfo {
            name: "exec".into(),
            input: "ls".into(),
        },
        HookToolCallInfo {
            name: "read".into(),
            input: r#"{"path":"/x"}"#.into(),
        },
    ];
    let ctx = build_hook_context(&result, recent);
    assert_eq!(ctx.recent_tool_calls.len(), 2);
    assert_eq!(ctx.recent_tool_calls[0].name, "exec");
    assert_eq!(ctx.recent_tool_calls[1].name, "read");
}

#[test]
fn test_build_hook_context_recent_calls_preserves_order() {
    let result = make_stream_result(vec![]);
    let recent = vec![
        HookToolCallInfo {
            name: "a".into(),
            input: "1".into(),
        },
        HookToolCallInfo {
            name: "b".into(),
            input: "2".into(),
        },
        HookToolCallInfo {
            name: "c".into(),
            input: "3".into(),
        },
    ];
    let ctx = build_hook_context(&result, recent);
    assert_eq!(ctx.recent_tool_calls[0].name, "a");
    assert_eq!(ctx.recent_tool_calls[1].name, "b");
    assert_eq!(ctx.recent_tool_calls[2].name, "c");
}
