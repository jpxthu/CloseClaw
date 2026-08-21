use super::*;

fn text_turn(content: &str) -> TurnResponse {
    TurnResponse {
        response: ResponseShape::Text(TextResponse {
            content: content.to_string(),
            usage: None,
        }),
        delay: None,
        error: None,
    }
}

fn features(model: &str, msg: &str) -> RequestFeatures {
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
    }
}

#[test]
fn decide_fallback_when_no_match() {
    let mut engine = ScenarioEngine::new(vec![]);
    let feat = features("gpt-4", "hello");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "default");
            assert_eq!(d.response_blocks.len(), 1);
        }
        DecisionOutcome::Error(_) => panic!("expected decision, got error"),
    }
}

#[test]
fn decide_matches_scenario_and_returns_turn() {
    let scenario = ScenarioDeclaration {
        name: "basic".to_string(),
        match_: None,
        turns: vec![text_turn("hello"), text_turn("world")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "basic");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("hello"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }

    // Second request with extended history -> turn 1
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
                content: "hello".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
    };
    let outcome2 = engine.decide(&feat2);
    match outcome2 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("world"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_error_injection() {
    let scenario = ScenarioDeclaration {
        name: "error-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: String::new(),
                usage: None,
            }),
            delay: None,
            error: Some(HttpError {
                status: 500,
                message: "server error".to_string(),
                retry_after: None,
            }),
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert_eq!(e.message, "server error");
        }
        DecisionOutcome::Decision(_) => panic!("expected error"),
    }
}

#[test]
fn decide_captures_usage() {
    let scenario = ScenarioDeclaration {
        name: "usage-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Usage(UsageResponse {
                prompt_tokens: Some(10),
                completion_tokens: Some(20),
                ..Default::default()
            }),
            delay: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let u = d.usage.unwrap();
            assert_eq!(u.prompt_tokens, Some(10));
            assert_eq!(u.completion_tokens, Some(20));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_captures_delay() {
    let scenario = ScenarioDeclaration {
        name: "delay-scene".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: "slow".to_string(),
                usage: None,
            }),
            delay: Some(500),
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => assert_eq!(d.delay, Some(500)),
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

// ------------------------------------------------------------------
// Fixture-loaded integration tests
// ------------------------------------------------------------------

/// Resolve the path to `tests/fixtures/fake_llm/scenarios/`.
fn fixture_scenarios_dir() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("fake_llm")
        .join("scenarios")
}

fn features_with_model(model: &str, msg: &str) -> RequestFeatures {
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
    }
}

fn features_with_messages(model: &str, messages: Vec<(&str, &str)>) -> RequestFeatures {
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
    }
}

#[test]
fn decide_end_to_end_from_dir() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // basic-text.json: greeting scenario matches model "gpt-4o-basic" + "hello"
    let feat = features_with_model("gpt-4o-basic", "hello world");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "greeting");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("Hi there! How can I help?")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_fallback_basic() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // basic-text.json: fallback-basic matches model "gpt-4o-basic-fallback"
    let feat = features_with_model("gpt-4o-basic-fallback", "something else");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "fallback-basic");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("I'm a fake LLM server.")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_error_injection_rate_limit() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // error-injection.json: rate-limit scenario — first turn OK, second turn 429
    let feat = features_with_model("gpt-4o-error", "hi");
    let outcome1 = engine.decide(&feat);
    match outcome1 {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "rate-limit");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("OK before error")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision on first turn"),
    }

    // Second request: same session -> error injection
    let feat2 = RequestFeatures {
        model: "gpt-4o-error".to_string(),
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
                content: "OK before error".to_string(),
            },
            MessageEntry {
                role: "user".to_string(),
                content: "next".to_string(),
            },
        ],
        tools: vec![],
    };
    let outcome2 = engine.decide(&feat2);
    match outcome2 {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 429);
            assert_eq!(e.message, "rate limit exceeded");
        }
        DecisionOutcome::Decision(_) => panic!("expected error on second turn"),
    }
}

#[test]
fn decide_fixture_error_injection_server_error() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // error-injection.json: server-error matches model "gpt-4o-error-search" + tool "web_search"
    let feat = RequestFeatures {
        model: "gpt-4o-error-search".to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![MessageEntry {
            role: "user".to_string(),
            content: "search something".to_string(),
        }],
        tools: vec!["web_search".to_string()],
    };
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Error(e) => {
            assert_eq!(e.status, 500);
            assert_eq!(e.message, "internal server error");
        }
        DecisionOutcome::Decision(_) => panic!("expected error"),
    }
}

#[test]
fn decide_fixture_multi_turn_turn1() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // multi-turn.json: three-turn-chat with model "gpt-4o-multi"
    let feat = features_with_model("gpt-4o-multi", "start");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "three-turn-chat");
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("Turn 1: Hello!")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_multi_turn_turn2() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // Drive to turn 1 first
    let feat1 = features_with_model("gpt-4o-multi", "start");
    let _ = engine.decide(&feat1);

    // Turn 2
    let feat = features_with_messages(
        "gpt-4o-multi",
        vec![
            ("user", "start"),
            ("assistant", "Turn 1: Hello!"),
            ("user", "continue"),
        ],
    );
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("Turn 2: How are you?")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_multi_turn_turn3() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // Drive to turn 2
    let feat1 = features_with_model("gpt-4o-multi", "start");
    let _ = engine.decide(&feat1);
    let feat2 = features_with_messages(
        "gpt-4o-multi",
        vec![
            ("user", "start"),
            ("assistant", "Turn 1: Hello!"),
            ("user", "continue"),
        ],
    );
    let _ = engine.decide(&feat2);

    // Turn 3
    let feat = features_with_messages(
        "gpt-4o-multi",
        vec![
            ("user", "start"),
            ("assistant", "Turn 1: Hello!"),
            ("user", "continue"),
            ("assistant", "Turn 2: How are you?"),
            ("user", "bye"),
        ],
    );
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("Turn 3: Goodbye!")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_usage_response() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // usage-response.json: usage-report with model "gpt-4o-usage"
    let feat = features_with_model("gpt-4o-usage", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "usage-report");
            let u = d.usage.unwrap();
            assert_eq!(u.prompt_tokens, Some(15));
            assert_eq!(u.completion_tokens, Some(30));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_fixture_cache_fields_missing() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // cache-fields-missing.json: no-cache-fields-vendor with model "vendor-no-cache"
    let feat = features_with_model("vendor-no-cache", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "no-cache-fields-vendor");
            let u = d.usage.unwrap();
            // cache_fields_missing=true => auto-simulated cache fields not filled
            assert!(
                u.cache_hit_tokens.is_none(),
                "cache_hit_tokens must be None"
            );
            assert!(
                u.cache_write_tokens.is_none(),
                "cache_write_tokens must be None"
            );
            // Basic usage fields are still present
            assert_eq!(u.prompt_tokens, Some(100));
            assert_eq!(u.completion_tokens, Some(50));
            // Content is correct
            assert_eq!(
                d.response_blocks[0].content.as_deref(),
                Some("Response from a vendor that does not return cache fields.")
            );
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
fn decide_unknown_model_returns_default() {
    let dir = fixture_scenarios_dir();
    let mut engine = ScenarioEngine::from_dir(&dir).unwrap();

    // No fixture matches model "unknown-model" -> default placeholder
    let feat = features_with_model("unknown-model", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.scenario, "default");
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("placeholder"));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

#[test]
#[should_panic(expected = "exceeded declared turns")]
fn decide_panics_on_turn_overflow() {
    let scenario = ScenarioDeclaration {
        name: "single-turn".to_string(),
        match_: None,
        turns: vec![text_turn("only one")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);

    // First request → turn 0, succeeds.
    let feat1 = features("gpt-4", "hi");
    let _ = engine.decide(&feat1);

    // Second request with extended history → turn 1, exceeds max 1.
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
    let _ = engine.decide(&feat2);
}

// ------------------------------------------------------------------
// decide_for_models tests
// ------------------------------------------------------------------
#[test]
fn decide_for_models_returns_scenario_declared_models() {
    let scenario = ScenarioDeclaration {
        name: "models-scene".to_string(),
        match_: None,
        turns: vec![text_turn("ok")],
        models: Some(vec![
            ModelEntry {
                id: "gpt-4".to_string(),
                owned_by: "openai".to_string(),
            },
            ModelEntry {
                id: "claude-3".to_string(),
                owned_by: "anthropic".to_string(),
            },
        ]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let decision = engine.decide_for_models();
    match decision {
        ModelsDecision::Models(entries, delay) => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].id, "gpt-4");
            assert_eq!(entries[1].id, "claude-3");
            assert!(delay.is_none());
        }
        _ => panic!("expected Models variant"),
    }
}
#[test]
fn decide_for_models_placeholder_when_no_models_declared() {
    let scenario = ScenarioDeclaration {
        name: "no-models".to_string(),
        match_: None,
        turns: vec![text_turn("ok")],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let decision = engine.decide_for_models();
    assert!(matches!(decision, ModelsDecision::Placeholder));
}
#[test]
fn decide_for_models_placeholder_when_no_scenarios() {
    let mut engine = ScenarioEngine::new(vec![]);
    let decision = engine.decide_for_models();
    assert!(matches!(decision, ModelsDecision::Placeholder));
}
#[test]
fn decide_for_models_error_injection() {
    let scenario = ScenarioDeclaration {
        name: "models-error".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: String::new(),
                usage: None,
            }),
            delay: None,
            error: Some(HttpError {
                status: 429,
                message: "rate limited".to_string(),
                retry_after: None,
            }),
        }],
        models: Some(vec![ModelEntry {
            id: "gpt-4".to_string(),
            owned_by: "openai".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let decision = engine.decide_for_models();
    match decision {
        ModelsDecision::Error(e) => {
            assert_eq!(e.status, 429);
            assert_eq!(e.message, "rate limited");
        }
        _ => panic!("expected Error variant"),
    }
}
#[test]
fn decide_for_models_returns_models_when_no_error() {
    let scenario = ScenarioDeclaration {
        name: "models-ok".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: String::new(),
                usage: None,
            }),
            delay: None,
            error: None,
        }],
        models: Some(vec![ModelEntry {
            id: "test-model".to_string(),
            owned_by: "test-org".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let decision = engine.decide_for_models();
    match decision {
        ModelsDecision::Models(entries, delay) => {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, "test-model");
            assert!(delay.is_none());
        }
        _ => panic!("expected Models variant"),
    }
}
#[test]
fn decide_for_models_with_model_id_constraint() {
    let scenario = ScenarioDeclaration {
        name: "gpt4-models".to_string(),
        match_: Some(types::MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        }),
        turns: vec![text_turn("ok")],
        models: Some(vec![types::ModelEntry {
            id: "gpt-4o".to_string(),
            owned_by: "openai".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    let decision = engine.decide_for_models();
    match decision {
        ModelsDecision::Models(entries, delay) => {
            assert_eq!(entries[0].id, "gpt-4o");
            assert!(delay.is_none());
        }
        _ => panic!("expected Models variant, got {:?}", decision),
    }
}
#[test]
fn decide_for_models_carrying_delay() {
    let scenario = ScenarioDeclaration {
        name: "delayed-models".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: String::new(),
                usage: None,
            }),
            delay: Some(500),
            error: None,
        }],
        models: Some(vec![types::ModelEntry {
            id: "m1".to_string(),
            owned_by: "org".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]);
    match engine.decide_for_models() {
        ModelsDecision::Models(_, delay) => assert_eq!(delay, Some(500)),
        d => panic!("expected Models variant with delay, got {:?}", d),
    }
}
mod fixture_contract;
mod response_blocks;
mod streaming_fixture_contract;
