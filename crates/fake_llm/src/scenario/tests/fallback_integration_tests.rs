//! Fallback behavior and integration tests.
//!
//! Tests the zero-hit → fallback path through `ScenarioEngine::decide`,
//! including multi-turn cursor advancement, empty shapes, and
//! cross-protocol routing.

use super::*;
use crate::scenario::types::{
    MatchCondition, ResponseShape, ScenarioDeclaration, TextResponse, TurnResponse,
};
use crate::types::ProtocolKind;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn text_turn(content: &str) -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Text(TextResponse {
            content: content.to_string(),
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

fn usage_turn() -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Usage(UsageResponse {
            prompt_tokens: Some(10),
            completion_tokens: Some(20),
            ..Default::default()
        })
        .into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

fn unknown_turn() -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Unknown.into(),
        delay: None,
        first_token_delay: None,
        segment_delay: None,
        stream_interrupt_after: None,
        error: None,
    }
}

fn feat(model: &str, msg: &str) -> RequestFeatures {
    feat_proto(model, msg, ProtocolKind::OpenAi)
}

fn feat_proto(model: &str, msg: &str, protocol: ProtocolKind) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![MessageEntry {
            role: "user".to_string(),
            content: msg.to_string(),
        }],
        tools: vec![],
        protocol,
    }
}

fn feat_multi(model: &str, messages: Vec<(&str, &str)>) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: messages
            .into_iter()
            .map(|(role, content)| MessageEntry {
                role: role.to_string(),
                content: content.to_string(),
            })
            .collect(),
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    }
}

// ===================================================================
// Zero-hit → fallback: multi-turn cursor advancement
// ===================================================================

#[test]
fn fallback_zero_hit_returns_turn0() {
    let fallback = ScenarioDeclaration {
        name: "fb".to_string(),
        match_: None,
        turns: vec![text_turn("t0"), text_turn("t1"), text_turn("t2")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![fallback]).unwrap();

    // Zero-hit: model "unknown" has no matching scenario → fallback.
    let outcome = engine.decide(&feat("unknown-model", "hi"));
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "fb");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t0"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision from fallback"),
    }
}

#[test]
fn fallback_multi_turn_cursor_advances() {
    let fallback = ScenarioDeclaration {
        name: "fb".to_string(),
        match_: None,
        turns: vec![text_turn("t0"), text_turn("t1"), text_turn("t2")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![fallback]).unwrap();

    // Turn 0
    let outcome0 = engine.decide(&feat("unknown", "hi"));
    match outcome0 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t0"));
        }
        _ => panic!("expected decision"),
    }

    // Turn 1: extend message history
    let outcome1 = engine.decide(&feat_multi(
        "unknown",
        vec![("user", "hi"), ("assistant", "t0"), ("user", "next")],
    ));
    match outcome1 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t1"));
        }
        _ => panic!("expected decision"),
    }

    // Turn 2
    let outcome2 = engine.decide(&feat_multi(
        "unknown",
        vec![
            ("user", "hi"),
            ("assistant", "t0"),
            ("user", "next"),
            ("assistant", "t1"),
            ("user", "bye"),
        ],
    ));
    match outcome2 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t2"));
        }
        _ => panic!("expected decision"),
    }
}

#[test]
fn fallback_turn_overflow_returns_error() {
    let fallback = ScenarioDeclaration {
        name: "fb".to_string(),
        match_: None,
        turns: vec![text_turn("t0")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![fallback]).unwrap();

    // Turn 0 succeeds
    let _ = engine.decide(&feat("unknown", "hi"));

    // Turn 1: exceeds declared turns → 500 error
    let outcome = engine.decide(&feat_multi(
        "unknown",
        vec![("user", "hi"), ("assistant", "t0"), ("user", "next")],
    ));
    match outcome {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(e.message.contains("exceeded declared turns"));
        }
        DecisionOutcome::Decision(_) => panic!("expected error for turn overflow"),
    }
}

// ===================================================================
// No-fallback zero-hit → 500
// ===================================================================

#[test]
fn no_fallback_zero_hit_returns_500() {
    let scenario = ScenarioDeclaration {
        name: "specific".to_string(),
        match_: Some(MatchCondition {
            model_id: Some("gpt-4o".into()),
            ..Default::default()
        }),
        turns: vec![text_turn("hello")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();

    // "unknown-model" doesn't match any scenario, no fallback declared.
    let outcome = engine.decide(&feat("unknown-model", "hi"));
    match outcome {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(e.message.contains("no fallback declared"));
        }
        DecisionOutcome::Decision(_) => panic!("expected 500 error"),
    }
}

#[test]
fn empty_engine_zero_hit_returns_500() {
    let mut engine = ScenarioEngine::new(vec![]).unwrap();
    let outcome = engine.decide(&feat("any", "hi"));
    match outcome {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(e.message.contains("no fallback declared"));
        }
        DecisionOutcome::Decision(_) => panic!("expected 500 error"),
    }
}

// ===================================================================
// Empty shape / usage-only shape → empty blocks
// ===================================================================

#[test]
fn usage_only_shape_produces_empty_blocks() {
    let scenario = ScenarioDeclaration {
        name: "usage-only".to_string(),
        match_: None,
        turns: vec![usage_turn()],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&feat("any", "hi"));
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert!(
                d.response_blocks.is_empty(),
                "usage-only shape should produce empty response blocks"
            );
            let u = d.usage.unwrap();
            assert_eq!(u.prompt_tokens, Some(10));
            assert_eq!(u.completion_tokens, Some(20));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn unknown_shape_produces_empty_text_block() {
    let scenario = ScenarioDeclaration {
        name: "unknown-shape".to_string(),
        match_: None,
        turns: vec![unknown_turn()],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&feat("any", "hi"));
    match outcome {
        DecisionOutcome::Decision(d) => {
            // Unknown shape produces a single text block with empty content.
            assert_eq!(d.response_blocks.len(), 1);
            assert_eq!(d.response_blocks[0].block_type, "text");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some(""));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ===================================================================
// No placeholder text injection
// ===================================================================

#[test]
fn no_placeholder_text_in_usage_only_response() {
    let scenario = ScenarioDeclaration {
        name: "usage-scene".to_string(),
        match_: None,
        turns: vec![usage_turn()],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&feat("any", "hi"));
    match outcome {
        DecisionOutcome::Decision(d) => {
            // Verify no block contains "placeholder" text
            for block in &d.response_blocks {
                if let Some(ref content) = block.content {
                    assert!(
                        !content.contains("placeholder"),
                        "response block must not contain placeholder text: {:?}",
                        content
                    );
                }
            }
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn no_placeholder_text_in_unknown_shape_response() {
    let scenario = ScenarioDeclaration {
        name: "unknown-scene".to_string(),
        match_: None,
        turns: vec![unknown_turn()],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let outcome = engine.decide(&feat("any", "hi"));
    match outcome {
        DecisionOutcome::Decision(d) => {
            for block in &d.response_blocks {
                if let Some(ref content) = block.content {
                    assert!(
                        !content.contains("placeholder"),
                        "response block must not contain placeholder text: {:?}",
                        content
                    );
                }
            }
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ===================================================================
// Cross-protocol fallback routing
// ===================================================================

#[test]
fn fallback_matches_openai_protocol() {
    let fallback = ScenarioDeclaration {
        name: "fb".to_string(),
        match_: None,
        turns: vec![text_turn("fallback-response")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![fallback]).unwrap();
    let outcome = engine.decide(&feat_proto("unknown", "hi", ProtocolKind::OpenAi));
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "fb");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("fallback-response")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn fallback_matches_anthropic_protocol() {
    let fallback = ScenarioDeclaration {
        name: "fb".to_string(),
        match_: None,
        turns: vec![text_turn("fallback-response")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![fallback]).unwrap();
    let outcome = engine.decide(&feat_proto("unknown", "hi", ProtocolKind::Anthropic));
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "fb");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("fallback-response")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn model_specific_scenario_does_not_leak_cross_protocol() {
    // Scenario only for gpt-4o. Anthropic request with same model name should
    // still match (the scenario has no protocol constraint in declaration,
    // so it's indexed under both protocols). But a different model should not.
    let scenario = ScenarioDeclaration {
        name: "gpt4-scene".to_string(),
        match_: Some(MatchCondition {
            model_id: Some("gpt-4o".into()),
            ..Default::default()
        }),
        turns: vec![text_turn("gpt4-response")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();

    // OpenAI request → matches
    match engine.decide(&feat_proto("gpt-4o", "hi", ProtocolKind::OpenAi)) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "gpt4-scene");
        }
        _ => panic!("expected decision"),
    }

    // Anthropic request with same model → also matches (no protocol constraint)
    match engine.decide(&feat_proto("gpt-4o", "hi", ProtocolKind::Anthropic)) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "gpt4-scene");
        }
        _ => panic!("expected decision"),
    }

    // Different model → no match, no fallback → 500
    match engine.decide(&feat_proto("claude-3", "hi", ProtocolKind::Anthropic)) {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
        }
        _ => panic!("expected error for unmatched model"),
    }
}

// ===================================================================
// Fallback session cursor is per-scenario (isolated from specific)
// ===================================================================

// ===================================================================
// Fallback session cursor isolation (two non-fallback scenarios)
// ===================================================================

/// Two conditional scenarios with different models have independent
/// session cursors — one matching model's cursor doesn't affect the other.
#[test]
fn specific_scenario_cursors_are_independent() {
    let scene_a = ScenarioDeclaration {
        name: "scene-a".to_string(),
        match_: Some(MatchCondition {
            model_id: Some("model-a".into()),
            ..Default::default()
        }),
        turns: vec![text_turn("a-t0"), text_turn("a-t1")],
        models: None,
    };
    let scene_b = ScenarioDeclaration {
        name: "scene-b".to_string(),
        match_: Some(MatchCondition {
            model_id: Some("model-b".into()),
            ..Default::default()
        }),
        turns: vec![text_turn("b-t0"), text_turn("b-t1")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scene_a, scene_b]).unwrap();

    // model-a → scene-a turn 0
    match engine.decide(&feat("model-a", "hi")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "scene-a");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("a-t0"));
        }
        _ => panic!("expected decision"),
    }

    // model-b → scene-b turn 0
    match engine.decide(&feat("model-b", "hi")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "scene-b");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("b-t0"));
        }
        _ => panic!("expected decision"),
    }

    // model-a again → scene-a turn 1 (cursor advanced independently)
    match engine.decide(&feat_multi(
        "model-a",
        vec![("user", "hi"), ("assistant", "a-t0"), ("user", "next")],
    )) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "scene-a");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("a-t1"));
        }
        _ => panic!("expected decision"),
    }

    // model-b again → scene-b turn 1 (cursor advanced independently)
    match engine.decide(&feat_multi(
        "model-b",
        vec![("user", "hi"), ("assistant", "b-t0"), ("user", "next")],
    )) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "scene-b");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("b-t1"));
        }
        _ => panic!("expected decision"),
    }
}
