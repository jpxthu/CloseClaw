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

fn make_m3_request(level: ReasoningLevel) -> InternalRequest {
    let mut req = make_request(level);
    req.model = "MiniMax-M3".into();
    req
}

fn make_m3_request_with_tools(level: ReasoningLevel, include_tool_result: bool) -> InternalRequest {
    let mut req = make_request_with_tools(level, include_tool_result);
    req.model = "MiniMax-M3".into();
    req
}

// ── name ──────────────────────────────────────────────────────────────

#[test]
fn test_name() {
    let plugin = MiniMaxPlugin;
    assert_eq!(plugin.name(), "minimax");
}

// ── reasoning_split tests ─────────────────────────────────────────────

#[test]
fn test_injects_reasoning_split_in_multiturn_tool_calls() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, true);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn test_preserves_existing_extra_body() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::Medium, true);
    req.extra_body.insert(
        "existing_key".to_string(),
        Value::String("existing_value".to_string()),
    );

    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        req.extra_body.get("existing_key"),
        Some(&Value::String("existing_value".to_string()))
    );
}

#[test]
fn test_no_tool_definitions_does_not_inject_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split without tool definitions"
    );
}

#[test]
fn test_tools_no_tool_results_does_not_inject_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, false);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split when there are no tool-result messages"
    );
}

#[test]
fn test_no_tools_with_tool_result_does_not_inject_reasoning_split() {
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

// ── M3 thinking: positive (normal paths) ─────────────────────────────

#[test]
fn test_m3_high_reasoning_injects_thinking() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 + High should inject thinking enabled"
    );
}

#[test]
fn test_m3_medium_reasoning_injects_thinking_disabled() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::Medium);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "disabled"})),
        "M3 + Medium should inject thinking disabled"
    );
}

#[test]
fn test_m3_low_reasoning_injects_thinking_disabled() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::Low);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "disabled"})),
        "M3 + Low should inject thinking disabled"
    );
}

#[test]
fn test_m3_max_reasoning_injects_thinking() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::Max);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 + Max should inject thinking enabled"
    );
}

#[test]
fn test_m3_default_reasoning_injects_thinking_enabled() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::default());
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 + default (High) should inject thinking enabled"
    );
}

// ── M3 thinking: negative (non-M3 models) ─────────────────────────────

#[test]
fn test_m27_does_not_inject_thinking() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    req.model = "MiniMax-M2.7".into();
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("thinking").is_none(),
        "M2.7 should not inject thinking"
    );
}

#[test]
fn test_regular_model_does_not_inject_thinking() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("thinking").is_none(),
        "non-M3 model should not inject thinking"
    );
}

// ── M3 thinking: combination (multi-turn tool calls) ──────────────────

#[test]
fn test_m3_multiturn_tool_calls_injects_thinking_and_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request_with_tools(ReasoningLevel::High, true);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 multi-turn should inject thinking"
    );
    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true)),
        "M3 multi-turn should also inject reasoning_split"
    );
}

#[test]
fn test_m3_multiturn_tool_calls_low_injects_thinking_disabled_and_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request_with_tools(ReasoningLevel::Low, true);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "disabled"})),
        "M3 multi-turn + Low should inject thinking disabled"
    );
    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true)),
        "M3 multi-turn + Low should also inject reasoning_split"
    );
}

// ── M3 thinking: variant prefix matching ───────────────────────────────

#[test]
fn test_m3_pro_variant_injects_thinking() {
    let plugin = MiniMaxPlugin;
    let mut req = make_m3_request(ReasoningLevel::High);
    req.model = "MiniMax-M3-Pro".into();
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "MiniMax-M3-Pro variant should inject thinking"
    );
}
