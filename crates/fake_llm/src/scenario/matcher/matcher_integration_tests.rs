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

use super::super::types::MessageEntry;
use super::*;
use crate::scenario::types::{MatchCondition, ResponseShape, TextResponse, TurnResponse};
use crate::types::ProtocolKind;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fallback(name: &str) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: None,
        turns: vec![turn()],
        models: None,
    }
}

fn specific(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: Some(condition),
        turns: vec![turn()],
        models: None,
    }
}

fn turn() -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Text(TextResponse {
            content: "ok".to_string(),
            usage: None,
        })
        .into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

fn feat(model: &str, messages: Vec<&str>, tools: Vec<&str>) -> RequestFeatures {
    feat_proto(model, messages, tools, ProtocolKind::OpenAi)
}

fn feat_proto(
    model: &str,
    messages: Vec<&str>,
    tools: Vec<&str>,
    protocol: ProtocolKind,
) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: messages
            .into_iter()
            .map(|c| MessageEntry {
                role: "user".to_string(),
                content: c.to_string(),
            })
            .collect(),
        tools: tools.into_iter().map(String::from).collect(),
        protocol,
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
fn integration_fallback_coexist_data_flow() {
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
    let hit = index.match_request(&feat("gpt-4o", vec![], vec![]));
    assert_eq!(index.get(hit.unwrap()).name, "conditional-gpt4");

    // Path 2: zero-match → fallback — request doesn't match any condition
    let miss = index.match_request(&feat("claude-3", vec![], vec![]));
    assert_eq!(index.get(miss.unwrap()).name, "fallback");
}

/// Multiple conditional scenarios with a fallback: each conditional
/// routes to its own scenario, and unmatched requests fall back.
#[test]
fn integration_multi_conditional_with_fallback() {
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
    let r = index.match_request(&feat("gpt-4o", vec![], vec![]));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // claude-3 → claude-scene
    let r = index.match_request(&feat("claude-3", vec![], vec![]));
    assert_eq!(index.get(r.unwrap()).name, "claude-scene");

    // unknown model → fallback
    let r = index.match_request(&feat("gemini-pro", vec![], vec![]));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

/// Cross-protocol: fallback + conditional coexist, each protocol
/// independently falls back when its conditional doesn't match.
#[test]
fn integration_cross_protocol_fallback_data_flow() {
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
    let r = index.match_request(&feat_proto("gpt-4o", vec![], vec![], ProtocolKind::OpenAi));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // OpenAI protocol: unknown model → fallback
    let r = index.match_request(&feat_proto("gemini", vec![], vec![], ProtocolKind::OpenAi));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Anthropic protocol: gpt-4o → conditional (indexed per-protocol)
    let r = index.match_request(&feat_proto(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");

    // Anthropic protocol: unknown model → fallback
    let r = index.match_request(&feat_proto(
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
fn integration_conditional_multi_field_vs_fallback() {
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
    let r = index.match_request(&feat(
        "gpt-4o",
        vec!["please calculate"],
        vec!["calculator"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "multi-cond");

    // Model matches but message doesn't → fallback
    let r = index.match_request(&feat("gpt-4o", vec!["hello world"], vec!["calculator"]));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Model and message match but tool doesn't → fallback
    let r = index.match_request(&feat("gpt-4o", vec!["please calculate"], vec![]));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    // Nothing matches → fallback
    let r = index.match_request(&feat("claude-3", vec!["hi"], vec![]));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}
