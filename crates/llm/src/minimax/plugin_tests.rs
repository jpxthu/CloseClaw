//! Unit tests for the MiniMax M3 and M2 plugins.

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

/// Build an `InternalRequest` with tool definitions and an optional
/// tool-result message.
///
/// When `include_tool_result` is true a message carrying `tool_call_id`
/// is appended, simulating a multi-turn tool-call scenario.
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

// ── applies_to routing ──────────────────────────────────────────────

#[test]
fn test_m3_plugin_name() {
    let plugin = MiniMaxM3Plugin;
    assert_eq!(plugin.name(), "minimax-m3");
}

#[test]
fn test_m2_plugin_name() {
    let plugin = MiniMaxM2Plugin;
    assert_eq!(plugin.name(), "minimax-m2");
}

#[test]
fn test_m3_plugin_applies_to_m3_models() {
    let plugin = MiniMaxM3Plugin;
    assert!(plugin.applies_to("MiniMax-M3"));
    assert!(plugin.applies_to("MiniMax-M3-Pro"));
    assert!(plugin.applies_to("MiniMax-M30"));
}

#[test]
fn test_m3_plugin_does_not_apply_to_m2_models() {
    let plugin = MiniMaxM3Plugin;
    assert!(!plugin.applies_to("MiniMax-M2.7"));
    assert!(!plugin.applies_to("minimax-model"));
}

#[test]
fn test_m2_plugin_applies_to_non_m3_minimax() {
    let plugin = MiniMaxM2Plugin;
    assert!(plugin.applies_to("MiniMax-M2.7"));
    assert!(plugin.applies_to("minimax-model"));
}

#[test]
fn test_m2_plugin_does_not_apply_to_m3_models() {
    let plugin = MiniMaxM2Plugin;
    assert!(!plugin.applies_to("MiniMax-M3"));
    assert!(!plugin.applies_to("MiniMax-M3-Pro"));
}

#[test]
fn test_m2_plugin_does_not_apply_to_non_minimax() {
    let plugin = MiniMaxM2Plugin;
    assert!(!plugin.applies_to("gpt-4"));
    assert!(!plugin.applies_to("claude-3"));
}

// ── M2 reasoning_split tests ───────────────────────────────────────

#[test]
fn test_m2_injects_reasoning_split_in_multiturn_tool_calls() {
    let plugin = MiniMaxM2Plugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, true);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn test_m2_preserves_existing_extra_body() {
    let plugin = MiniMaxM2Plugin;
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
fn test_m2_no_tool_definitions_does_not_inject() {
    let plugin = MiniMaxM2Plugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split without tool definitions"
    );
}

#[test]
fn test_m2_tools_no_tool_results_does_not_inject() {
    let plugin = MiniMaxM2Plugin;
    let mut req = make_request_with_tools(ReasoningLevel::High, false);
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "should not inject reasoning_split when no tool-result messages"
    );
}

#[test]
fn test_m2_no_tools_with_tool_result_does_not_inject() {
    let plugin = MiniMaxM2Plugin;
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

#[test]
fn test_m2_does_not_inject_thinking() {
    let plugin = MiniMaxM2Plugin;
    let mut req = make_request(ReasoningLevel::High);
    req.model = "MiniMax-M2.7".into();
    plugin.before_request(&mut req);

    assert!(
        req.extra_body.get("thinking").is_none(),
        "M2.7 should not inject thinking"
    );
}

// ── M3 thinking: positive (normal paths) ─────────────────────────────

#[test]
fn test_m3_high_reasoning_injects_thinking() {
    let plugin = MiniMaxM3Plugin;
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
    let plugin = MiniMaxM3Plugin;
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
    let plugin = MiniMaxM3Plugin;
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
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::Max);
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 + Max should inject thinking enabled (after downgrade to High)"
    );
}

#[test]
fn test_m3_max_downgrades_to_high() {
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::Max);
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
}

#[test]
fn test_m3_max_downgrade_triggers_logging_path() {
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::Max);
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"}))
    );
}

#[test]
fn test_m3_high_no_downgrade() {
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
}

#[test]
fn test_m3_default_reasoning_injects_thinking_enabled() {
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::default());
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "M3 + default (High) should inject thinking enabled"
    );
}

// ── M3 thinking: negative (non-M3 models) ─────────────────────────────
// NOTE: M3Plugin.applies_to("MiniMax-M2.7") returns false, so the pipeline
// never calls before_request for M2.7. Pipeline-level tests in
// test_pipeline_routes_m2_to_m2_plugin_only and
// test_pipeline_non_minimax_skips_both_plugins cover this filtering.

// ── M3 thinking: combination (multi-turn tool calls) ──────────────────

#[test]
fn test_m3_multiturn_tool_calls_injects_thinking_and_reasoning_split() {
    let plugin = MiniMaxM3Plugin;
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
fn test_m3_multiturn_tool_calls_low_injects_disabled_and_reasoning_split() {
    let plugin = MiniMaxM3Plugin;
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
    let plugin = MiniMaxM3Plugin;
    let mut req = make_m3_request(ReasoningLevel::High);
    req.model = "MiniMax-M3-Pro".into();
    plugin.before_request(&mut req);

    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"})),
        "MiniMax-M3-Pro variant should inject thinking"
    );
}

// ── applies_to + pipeline integration ─────────────────────────────────

use crate::plugin::PluginPipeline;

#[test]
fn test_pipeline_routes_m3_to_m3_plugin_only() {
    let pipeline = PluginPipeline::new()
        .add(Box::new(MiniMaxM3Plugin))
        .add(Box::new(MiniMaxM2Plugin));

    let mut req = make_m3_request(ReasoningLevel::High);
    pipeline.before_request(&mut req, "MiniMax-M3");

    // M3 plugin should inject thinking
    assert_eq!(
        req.extra_body.get("thinking"),
        Some(&json!({"type": "enabled"}))
    );
    // No tool defs → no reasoning_split
    assert!(req.extra_body.get("reasoning_split").is_none());
}

#[test]
fn test_pipeline_routes_m2_to_m2_plugin_only() {
    let pipeline = PluginPipeline::new()
        .add(Box::new(MiniMaxM3Plugin))
        .add(Box::new(MiniMaxM2Plugin));

    let mut req = make_request_with_tools(ReasoningLevel::High, true);
    req.model = "MiniMax-M2.7".into();
    pipeline.before_request(&mut req, "MiniMax-M2.7");

    // M2 plugin should inject reasoning_split
    assert_eq!(
        req.extra_body.get("reasoning_split"),
        Some(&Value::Bool(true))
    );
    // M3 plugin should NOT inject thinking for M2.7
    assert!(req.extra_body.get("thinking").is_none());
}

#[test]
fn test_pipeline_non_minimax_skips_both_plugins() {
    let pipeline = PluginPipeline::new()
        .add(Box::new(MiniMaxM3Plugin))
        .add(Box::new(MiniMaxM2Plugin));

    let mut req = make_request(ReasoningLevel::High);
    pipeline.before_request(&mut req, "gpt-4");

    assert!(req.extra_body.get("thinking").is_none());
    assert!(req.extra_body.get("reasoning_split").is_none());
}
