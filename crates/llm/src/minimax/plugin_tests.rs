//! Unit tests for the MiniMax plugin.

use super::*;
use crate::types::{InternalMessage, ToolDefinition};
use closeclaw_session::persistence::ReasoningLevel;

fn make_request(level: ReasoningLevel) -> InternalRequest {
    InternalRequest {
        model: "minimax-model".into(),
        messages: vec![],
        temperature: 0.0,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: level,
        turn_count: None,
    }
}

/// Build an `InternalRequest` with tool definitions and an optional tool-result message.
///
/// When `include_tool_result` is true a message carrying `tool_call_id` is appended,
/// simulating a multi-turn tool-call scenario.
fn make_request_with_tools(level: ReasoningLevel, include_tool_result: bool) -> InternalRequest {
    let tools = Some(vec![ToolDefinition {
        name: "get_weather".into(),
        description: "Get weather info".into(),
        input_schema: None,
        cache: false,
    }]);
    let mut messages = vec![];
    if include_tool_result {
        messages.push(InternalMessage {
            role: "tool".into(),
            content: "sunny, 25°C".into(),
            tool_call_id: Some("call_001".into()),
        });
    }
    InternalRequest {
        model: "minimax-model".into(),
        messages,
        temperature: 0.0,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools,
        session_id: None,
        reasoning_level: level,
        turn_count: None,
    }
}

// ── name ──────────────────────────────────────────────────────────────

#[test]
fn test_name() {
    let plugin = MiniMaxPlugin;
    assert_eq!(plugin.name(), "minimax");
}

// ── existing test (updated) ───────────────────────────────────────────

#[test]
fn test_before_request_injects_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, true);
    plugin.before_request(&mut req);

    let value = req.extra_body.get("reasoning_split");
    assert_eq!(value, Some(&Value::Bool(true)));
}

#[test]
fn test_before_request_preserves_existing_extra_body() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::Medium, true);
    req.extra_body.insert(
        "existing_key".to_string(),
        Value::String("existing_value".to_string()),
    );

    plugin.before_request(&mut req);

    // New field injected
    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true))
    );
    // Existing field preserved
    assert_eq!(
        req.extra_body.get("existing_key"),
        Some(&Value::String("existing_value".to_string()))
    );
}

// ── negative: no tool definitions ─────────────────────────────────────

#[test]
fn test_no_tool_definitions_does_not_inject() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split without tool definitions"
    );
}

// ── negative: tools present but no tool-result messages (single-turn) ─

#[test]
fn test_tools_no_tool_results_does_not_inject() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, false);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split when there are no tool-result messages"
    );
}

// ── positive: tools + tool-result messages (multi-turn) ───────────────

#[test]
fn test_tools_with_tool_results_injects() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, true);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true)),
        "should inject reasoning_split in multi-turn tool-call scenario"
    );
}

// ── edge: no tools but tool-result messages present ────────────────────

#[test]
fn test_no_tools_with_tool_result_does_not_inject() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    req.messages.push(InternalMessage {
        role: "tool".into(),
        content: "some result".into(),
        tool_call_id: Some("call_002".into()),
    });
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split when tool definitions are absent"
    );
}
