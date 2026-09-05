//! Unit tests for the DeepSeek plugin.

use super::*;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;

fn make_request(level: ReasoningLevel) -> InternalRequest {
    InternalRequest {
        model: "deepseek-reasoner".into(),
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

#[test]
fn test_name() {
    assert_eq!(DeepSeekPlugin.name(), "deepseek");
}

#[test]
fn test_low_maps_to_low() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Low);
    plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("low".into()))
    );
}

#[test]
fn test_medium_maps_to_base() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Medium);
    plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("base".into()))
    );
}

#[test]
fn test_high_maps_to_high() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("high".into()))
    );
}

#[test]
fn test_max_stays_max_and_maps_to_high() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Max);
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    plugin.before_request(&mut req);
    // Max stays Max — no downgrade; maps to "high" effort
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("high".into()))
    );
}

#[test]
fn test_high_no_downgrade() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("high".into()))
    );
}
