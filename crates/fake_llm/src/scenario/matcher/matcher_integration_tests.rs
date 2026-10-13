//! Integration-level verification tests for fallback coexistence data flow.
//!
//! Proves the design doc data flow:
//!   特征匹配 → 命中唯一场景（多命中为场景文件错误，零命中走兜底场景）
//!
//! These tests verify that:
//! - `MatcherIndex::build` succeeds when fallback + conditional scenarios coexist
//! - Conditional match routes to the conditional scenario (not fallback)
//! - Zero conditional match routes to the fallback scenario
//! - The behavior holds across protocols

use super::*;
use crate::scenario::types::MatchCondition;
use crate::test_util::{features, features_full, features_with_tools, text_turn};
use crate::types::ProtocolKind;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fallback(name: &str) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: None,
        turns: vec![text_turn("ok")],
        models: None,
    }
}

fn specific(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: Some(condition),
        turns: vec![text_turn("ok")],
        models: None,
    }
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

/// Verify the full data flow described in the design doc:
///   "特征匹配 → 命中唯一场景（多命中为场景文件错误，零命中走兜底场景）"
///
/// Scenario setup: one conditional (model=gpt-4o) + one fallback.
/// - gpt-4o request  → conditional (not fallback)
/// - claude-3 request → fallback (zero conditional match)
#[test]
fn test_integration_fallback_coexist_data_flow() {
    let scenarios = vec![
        specific(
            "conditional-gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        fallback("fallback"),
    ];

    // Build must succeed — fallback + conditional is legal
    let index = MatcherIndex::build(scenarios).unwrap();

    // Path 1: conditional hit — request matches gpt-4o condition
    let hit = index.match_request(&features_full(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(hit.unwrap()).name, "conditional-gpt4");

    // Path 2: zero-match → fallback — request doesn't match any condition
    let miss = index.match_request(&features_full(
        "claude-3",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(miss.unwrap()).name, "fallback");
}

/// Multiple conditional scenarios with a fallback: each conditional
/// routes to its own scenario, and unmatched requests fall back.
#[test]
fn test_integration_multi_conditional_with_fallback() {
    let scenarios = vec![
        specific(
            "gpt4-scene",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        specific(
            "claude-scene",
            MatchCondition {
                model_id: Some("claude-3".to_string()),
                ..Default::default()
            },
        ),
        fallback("fallback"),
    ];

    let index = MatcherIndex::build(scenarios).unwrap();

    // gpt-4o → gpt4-scene
    let r = index.match_request(&features_full(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // claude-3 → claude-scene
    let r = index.match_request(&features_full(
        "claude-3",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "claude-scene");

    // unknown model → fallback
    let r = index.match_request(&features_full(
        "gemini-pro",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

/// Cross-protocol: fallback + conditional coexist, each protocol
/// independently falls back when its conditional doesn't match.
#[test]
fn test_integration_cross_protocol_fallback_data_flow() {
    let scenarios = vec![
        specific(
            "gpt4-scene",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        fallback("fallback"),
    ];

    let index = MatcherIndex::build(scenarios).unwrap();

    // OpenAI protocol: gpt-4o → conditional
    let r = index.match_request(&features_full(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // OpenAI protocol: unknown model → fallback
    let r = index.match_request(&features_full(
        "gemini",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Anthropic protocol: gpt-4o → conditional (indexed per-protocol)
    let r = index.match_request(&features_full(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // Anthropic protocol: unknown model → fallback
    let r = index.match_request(&features_full(
        "gemini",
        vec![],
        vec![],
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

/// Conditional with multiple fields (model + message_contains + tool):
/// partial match is NOT enough — only exact conditional match or fallback.
#[test]
fn test_integration_conditional_multi_field_vs_fallback() {
    let scenarios = vec![
        specific(
            "multi-cond",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                message_contains: Some("calculate".to_string()),
                tool_name: Some("calculator".to_string()),
                ..Default::default()
            },
        ),
        fallback("fallback"),
    ];

    let index = MatcherIndex::build(scenarios).unwrap();

    // All three fields match → conditional
    let r = index.match_request(&features_with_tools(
        "gpt-4o",
        vec!["please calculate"],
        vec!["calculator"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "multi-cond");

    // Model matches but message doesn't → fallback
    let r = index.match_request(&features_with_tools(
        "gpt-4o",
        vec!["hello world"],
        vec!["calculator"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Model and message match but tool doesn't → fallback
    let r = index.match_request(&features("gpt-4o", "please calculate"));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Nothing matches → fallback
    let r = index.match_request(&features("claude-3", "hi"));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}
