//! Tests for structured turn overflow errors (Step 1.2).

use super::*;

#[test]
fn decide_returns_error_on_turn_overflow() {
    let scenario = ScenarioDeclaration {
        name: "single-turn".to_string(),
        match_: None,
        turns: vec![text_turn("only one")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // First request -> turn 0, succeeds.
    let feat1 = features("gpt-4", "hi");
    let _ = engine.decide(&feat1);

    // Second request with extended history -> turn 1, exceeds max 1.
    let feat2 = RequestFeatures {
        model: "gpt-4".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".to_string(),
                content: "hi".to_string(),
            },
            MessageEntry {
                role: "assistant".to_string(),
                content: "only one".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
    };
    match engine.decide(&feat2) {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(e.message.contains("single-turn"));
            assert!(e.message.contains("exceeded declared turns"));
        }
        DecisionOutcome::Decision(_) => panic!("expected Error on turn overflow"),
    }
}

#[test]
fn decide_error_includes_scenario_name_and_turn_info() {
    let scenario = ScenarioDeclaration {
        name: "named-scenario".to_string(),
        match_: None,
        turns: vec![text_turn("first")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // First request -> turn 0
    let feat1 = features("gpt-4", "go");
    let _ = engine.decide(&feat1);

    // Second request -> exceeds max 1
    let feat2 = RequestFeatures {
        model: "gpt-4".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".to_string(),
                content: "go".to_string(),
            },
            MessageEntry {
                role: "assistant".to_string(),
                content: "first".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
    };
    match engine.decide(&feat2) {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(
                e.message.contains("named-scenario"),
                "should contain scenario name"
            );
            assert!(e.message.contains("turn 1"), "should contain current turn");
            assert!(e.message.contains("max 1"), "should contain max turns");
        }
        DecisionOutcome::Decision(_) => panic!("expected Error"),
    }
}

#[test]
fn decide_for_models_returns_error_on_turn_overflow() {
    let scenario = ScenarioDeclaration {
        name: "models-single".to_string(),
        match_: None,
        turns: vec![text_turn("only")],
        models: Some(vec![ModelEntry {
            id: "gpt-4".to_string(),
            owned_by: "openai".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // First decide_for_models call -> turn 0, returns models
    let d1 = engine.decide_for_models();
    assert!(matches!(d1, ModelsDecision::Models(_, _)));

    // Drive turn via decide() with extended history -> advances to turn 1
    let feat = RequestFeatures {
        model: "gpt-4".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".to_string(),
                content: "".to_string(),
            },
            MessageEntry {
                role: "assistant".to_string(),
                content: "only".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
    };
    let _ = engine.decide(&feat);

    // Now decide_for_models -> turn 1, exceeds max 1
    match engine.decide_for_models() {
        ModelsDecision::Error(e) => {
            assert_eq!(e.status, 500);
            assert!(e.message.contains("models-single"));
            assert!(e.message.contains("exceeded declared turns"));
        }
        _ => panic!("expected Error on turn overflow"),
    }
}
