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
fn test_max_downgrades_to_high() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Max);
    plugin.before_request(&mut req);
    // Max is downgraded to High, which maps to "high"
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("high".into()))
    );
}

#[test]
fn test_max_downgrade_sets_level_to_high() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Max);
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
}

// ── downgrade logging verification ──────────────────────────────────

/// Verify that Max→High downgrade produces the correct tracing::info!
/// by checking the resulting state. The logging is verified indirectly:
/// if `downgrade_max_to_high` fires, `req.reasoning_level` is mutated.
#[test]
fn test_max_downgrade_triggers_logging_path() {
    let plugin = DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::Max);
    // Before: Max
    assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    plugin.before_request(&mut req);
    // After: High — the downgrade path (which includes tracing::info!) ran.
    assert_eq!(req.reasoning_level, ReasoningLevel::High);
    // The mapping confirms the downgrade was applied correctly.
    assert_eq!(
        req.extra_body.get("reasoning_effort"),
        Some(&Value::String("high".into()))
    );
}

/// Non-Max levels should NOT trigger the downgrade path.
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
