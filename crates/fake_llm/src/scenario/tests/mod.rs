use super::*;
use crate::types::ProtocolKind;

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
        protocol: ProtocolKind::OpenAi,
    }
}

#[test]
fn decide_fallback_when_no_match() {
    let mut engine = ScenarioEngine::new(vec![]).unwrap();
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();

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
        protocol: ProtocolKind::OpenAi,
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
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: Some(HttpError {
                status: 500,
                message: "server error".to_string(),
                retry_after: None,
            }),
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
            })
            .into(),
            delay: Some(500),
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
        protocol: ProtocolKind::OpenAi,
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
        protocol: ProtocolKind::OpenAi,
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
        protocol: ProtocolKind::OpenAi,
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
        protocol: ProtocolKind::OpenAi,
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let decision = engine.decide_for_models();
    assert!(matches!(decision, ModelsDecision::Placeholder));
}
#[test]
fn decide_for_models_placeholder_when_no_scenarios() {
    let mut engine = ScenarioEngine::new(vec![]).unwrap();
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
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: Some(vec![ModelEntry {
            id: "test-model".to_string(),
            owned_by: "test-org".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
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
            })
            .into(),
            delay: Some(500),
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: Some(vec![types::ModelEntry {
            id: "m1".to_string(),
            owned_by: "org".to_string(),
        }]),
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    match engine.decide_for_models() {
        ModelsDecision::Models(_, delay) => assert_eq!(delay, Some(500)),
        d => panic!("expected Models variant with delay, got {:?}", d),
    }
}
// ------------------------------------------------------------------
// Step 1.3: Per-scenario isolation tests
// ------------------------------------------------------------------

/// Two scenarios with different models share the same message prefix
/// but should have independent session cursors and KV cache state.
#[test]
fn per_scenario_isolation_cursors_and_cache() {
    let prefix = vec![MessageEntry {
        role: "user".to_string(),
        content: "hello".to_string(),
    }];
    let mk_feat = |model: &str| RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: prefix.clone(),
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    };
    let mk_feat_ext = |model: &str, reply: &str| RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![
            MessageEntry {
                role: "user".into(),
                content: "hello".into(),
            },
            MessageEntry {
                role: "assistant".into(),
                content: reply.to_string(),
            },
            MessageEntry {
                role: "user".into(),
                content: "next".into(),
            },
        ],
        tools: vec![],
        protocol: ProtocolKind::OpenAi,
    };
    let decl = |name: &str, model: &str| ScenarioDeclaration {
        name: name.to_string(),
        match_: Some(types::MatchCondition {
            model_id: Some(model.to_string()),
            ..Default::default()
        }),
        turns: vec![text_turn("t0"), text_turn("t1"), text_turn("t2")],
        models: None,
    };
    let mut engine =
        ScenarioEngine::new(vec![decl("scene-a", "model-a"), decl("scene-b", "model-b")]).unwrap();

    // Scene A turn 0, Scene B turn 0 (independent cursors)
    match engine.decide(&mk_feat("model-a")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t0"))
        }
        _ => panic!("expected Decision"),
    }
    match engine.decide(&mk_feat("model-b")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t0"))
        }
        _ => panic!("expected Decision"),
    }
    // Scene A turn 1, Scene B turn 1 (cursors advance independently)
    match engine.decide(&mk_feat_ext("model-a", "t0")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t1"))
        }
        _ => panic!("expected Decision"),
    }
    match engine.decide(&mk_feat_ext("model-b", "t0")) {
        DecisionOutcome::Decision(d) => {
            assert_eq!(d.response_blocks[0].content.as_deref(), Some("t1"))
        }
        _ => panic!("expected Decision"),
    }
    // KV cache: each scenario has its own simulator instance.
    assert!(engine.kv_caches.contains_key("scene-a"));
    assert!(engine.kv_caches.contains_key("scene-b"));
}

/// Target 3: Explicit injection takes priority over cache_fields_missing.
/// When cache_fields_missing=true but cache_hit_tokens is explicitly set,
/// the explicit value is returned (100), field missing declaration is ignored.
#[test]
fn cache_fields_missing_with_explicit_injection_priority() {
    let scenario = ScenarioDeclaration {
        name: "cache-missing-explicit".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: "response".to_string(),
                usage: Some(UsageResponse {
                    prompt_tokens: Some(100),
                    completion_tokens: Some(50),
                    reasoning_tokens: None,
                    cache_hit_tokens: Some(100),
                    cache_write_tokens: None,
                    cache_fields_missing: true,
                }),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let u = d.usage.unwrap();
            // Explicit injection priority: cache_hit_tokens=100 wins
            assert_eq!(u.cache_hit_tokens, Some(100));
            // cache_fields_missing=true but explicit wins, so write is None
            assert!(u.cache_write_tokens.is_none());
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Target 3 variant: explicit injection for both hit and write with
/// cache_fields_missing=true — both explicit values survive.
#[test]
fn cache_fields_missing_with_explicit_both_fields() {
    let scenario = ScenarioDeclaration {
        name: "cache-missing-explicit-both".to_string(),
        match_: None,
        turns: vec![TurnResponse {
            response: ResponseShape::Text(TextResponse {
                content: "response".to_string(),
                usage: Some(UsageResponse {
                    prompt_tokens: Some(100),
                    completion_tokens: Some(50),
                    reasoning_tokens: None,
                    cache_hit_tokens: Some(100),
                    cache_write_tokens: Some(200),
                    cache_fields_missing: true,
                }),
            })
            .into(),
            delay: None,
            first_token_delay: None,
            segment_delay: None,
            stream_interrupt_after: None,
            error: None,
        }],
        models: None,
    };
    let mut engine = ScenarioEngine::new(vec![scenario]).unwrap();
    let feat = features("gpt-4", "hi");
    let outcome = engine.decide(&feat);
    match outcome {
        DecisionOutcome::Decision(d) => {
            let u = d.usage.unwrap();
            // Both explicit values preserved despite cache_fields_missing
            assert_eq!(u.cache_hit_tokens, Some(100));
            assert_eq!(u.cache_write_tokens, Some(200));
        }
        DecisionOutcome::Error(_) => panic!("expected decision"),
    }
}

/// Build a `Vec<MessageEntry>` from role/content pairs.
fn make_entries(pairs: &[(&str, &str)]) -> Vec<MessageEntry> {
    pairs
        .iter()
        .map(|(role, content)| MessageEntry {
            role: role.to_string(),
            content: content.to_string(),
        })
        .collect()
}

/// Process a prefix through the KV cache simulator, merge into a
/// `UsageResponse` with the given `cache_fields_missing` flag, and
/// return the merged usage.
fn build_and_merge_usage(
    sim: &mut crate::kv_cache::KvCacheSimulator,
    pairs: &[(&str, &str)],
    cache_fields_missing: bool,
) -> UsageResponse {
    let entries = make_entries(pairs);
    let cache = sim.process("test", &entries, &[], None, None);
    let mut usage = Some(UsageResponse {
        prompt_tokens: Some(100),
        completion_tokens: Some(50),
        cache_fields_missing,
        ..Default::default()
    });
    ScenarioEngine::merge_cache_into_usage(&mut usage, &cache);
    usage.unwrap()
}

/// Target 4: State machine continuity — after cache_fields_missing=true,
/// switching back to auto-sim still computes correct cache values.
#[test]
fn state_machine_continuity_after_cache_fields_missing() {
    let mut sim = crate::kv_cache::KvCacheSimulator::new();
    let prefix1: &[(&str, &str)] = &[
        ("system", "sys"),
        ("user", "hello"),
        ("assistant", "hi"),
        ("user", "q1"),
    ];
    let prefix2: &[(&str, &str)] = &[
        ("system", "sys"),
        ("user", "hello"),
        ("assistant", "hi"),
        ("user", "q2"),
    ];

    // Request 1: cache_fields_missing=true → fields not filled
    let u1 = build_and_merge_usage(&mut sim, prefix1, true);
    assert!(u1.cache_hit_tokens.is_none(), "no fill when missing");
    assert!(u1.cache_write_tokens.is_none(), "no fill when missing");

    // Request 2: same prefix, cache_fields_missing=false → auto-sim hit
    let u2 = build_and_merge_usage(&mut sim, prefix2, false);
    assert!(u2.cache_hit_tokens.is_some(), "same prefix → cache hit");
    assert!(u2.cache_hit_tokens.unwrap() > 0, "hit tokens positive");
    assert!(u2.cache_write_tokens.is_none(), "same prefix → no write");
}

/// Target 4 variant: cache_fields_missing=true then switch to auto-sim
/// with a different prefix — break with write tokens.
#[test]
fn state_machine_continuity_break_after_cache_fields_missing() {
    let mut sim = crate::kv_cache::KvCacheSimulator::new();
    let prefix_a: &[(&str, &str)] = &[
        ("system", "sys"),
        ("user", "hello"),
        ("assistant", "hi"),
        ("user", "q1"),
    ];
    let prefix_b: &[(&str, &str)] = &[
        ("system", "new sys"),
        ("user", "world"),
        ("assistant", "yo"),
        ("user", "q2"),
    ];

    // Request 1: cache_fields_missing=true → fields not filled
    let u1 = build_and_merge_usage(&mut sim, prefix_a, true);
    assert!(u1.cache_hit_tokens.is_none());
    assert!(u1.cache_write_tokens.is_none());

    // Request 2: different prefix, cache_fields_missing=false → write tokens
    let u2 = build_and_merge_usage(&mut sim, prefix_b, false);
    assert!(
        u2.cache_write_tokens.is_some(),
        "different prefix → cache write"
    );
    assert!(u2.cache_write_tokens.unwrap() > 0, "write tokens positive");
}

mod fixture_contract;
mod reasoning_intensity;
mod response_blocks;
mod session_cleanup_integration;
mod streaming_fixture_contract;
mod turn_overflow;
