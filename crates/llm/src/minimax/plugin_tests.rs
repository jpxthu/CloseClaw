//! Unit tests for the MiniMax plugin.

use super::*;
use crate::types::InternalRequest;
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

#[test]
fn test_name() {
    let plugin = MiniMaxPlugin;
    assert_eq!(plugin.name(), "minimax");
}

#[test]
fn test_before_request_injects_reasoning_split() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);

    let value = req.extra_body.get("reasoning_split");
    assert_eq!(value, Some(&Value::Bool(true)));
}

#[test]
fn test_before_request_preserves_existing_extra_body() {
    let plugin = MiniMaxPlugin;
    let mut req = make_request(ReasoningLevel::Medium);
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
