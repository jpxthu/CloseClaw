//! Unit tests for the DeepSeek plugin.

use super::*;
use crate::types::InternalRequest;

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
fn test_low_maps_to_off() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Low);
    plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("off".into()))
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
fn test_max_maps_to_reasoner() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Max);
    plugin.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("reasoner".into()))
    );
}
