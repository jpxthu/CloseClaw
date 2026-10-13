use super::*;
use crate::test_util::{
    features, features_full, features_multi, features_with_protocol, features_with_tools, text_turn,
};
use crate::types::ProtocolKind;

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

// ------------------------------------------------------------------
// MatcherIndex tests
// ------------------------------------------------------------------

#[test]
fn test_index_exact_model_match() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("gpt-4o", "hi"));
    assert_eq!(index.get(result.unwrap()).name, "gpt4");
}

#[test]
fn test_index_model_mismatch_returns_none() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("claude-3", "hi"));
    assert!(result.is_none());
}

#[test]
fn test_index_message_contains_match() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("gpt-4o", "please calculate 2+2"));
    assert_eq!(index.get(result.unwrap()).name, "math");
}

#[test]
fn test_index_message_contains_no_match() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("gpt-4o", "hello world"));
    assert!(result.is_none());
}

#[test]
fn test_index_tool_name_match() {
    let scenarios = vec![specific(
        "web_search",
        MatchCondition {
            tool_name: Some("search".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_with_tools(
        "gpt-4o",
        vec!["hi"],
        vec!["search", "math"],
    ));
    assert_eq!(index.get(result.unwrap()).name, "web_search");
}

#[test]
fn test_index_tool_name_no_match() {
    let scenarios = vec![specific(
        "web_search",
        MatchCondition {
            tool_name: Some("search".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_with_tools("gpt-4o", vec!["hi"], vec!["math"]));
    assert!(result.is_none());
}

#[test]
fn test_index_combined_conditions_all_must_match() {
    let scenarios = vec![specific(
        "specific",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            message_contains: Some("hello".to_string()),
            tool_name: Some("search".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r = index.match_request(&features_with_tools(
        "gpt-4o",
        vec!["hello world"],
        vec!["search"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "specific");

    assert!(index
        .match_request(&features_with_tools(
            "claude-3",
            vec!["hello world"],
            vec!["search"]
        ))
        .is_none());
    assert!(index
        .match_request(&features_with_tools(
            "gpt-4o",
            vec!["goodbye"],
            vec!["search"]
        ))
        .is_none());
    assert!(index
        .match_request(&features_with_tools(
            "gpt-4o",
            vec!["hello world"],
            vec!["math"],
        ))
        .is_none());
}

#[test]
fn test_index_fallback_matches_any() {
    let scenarios = vec![fallback("fallback")];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("any-model", "anything"));
    assert_eq!(index.get(result.unwrap()).name, "fallback");
}

#[test]
fn test_index_fallback_and_specific_coexist() {
    // Fallback + conditional with model_id = legal (fallback is the
    // zero-match兜底, checked only after conditionals fail).
    let scenarios = vec![
        specific(
            "specific",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        fallback("fallback"),
    ];
    let result = MatcherIndex::build(scenarios);
    assert!(result.is_ok());
    let index = result.unwrap();
    // Request matching specific scenario hits it directly
    let r = index.match_request(&features("gpt-4o", "hi"));
    assert_eq!(index.get(r.unwrap()).name, "specific");
    // Request with non-matching model falls back
    let r = index.match_request(&features("claude-3", "hi"));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

#[test]
fn test_index_multiple_specific_same_model_is_conflict() {
    // Two scenarios with same model_id = conflict at build time.
    let scenarios = vec![
        specific(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        specific(
            "b",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
    ];
    let result = MatcherIndex::build(scenarios);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.conflicts.len(), 1);
}

#[test]
fn test_index_no_match_returns_none() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            message_contains: Some("special".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features("claude-3", "hi"));
    assert!(result.is_none());
}

#[test]
fn test_index_empty_returns_none() {
    let index = MatcherIndex::build(vec![]).unwrap();
    let result = index.match_request(&features("gpt-4o", "hi"));
    assert!(result.is_none());
}

#[test]
fn test_index_message_contains_checks_all_messages() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_multi(
        "gpt-4o",
        vec![("user", "first message"), ("user", "please calculate this")],
    ));
    assert_eq!(index.get(result.unwrap()).name, "math");
}

#[test]
fn test_index_multiple_models_indexed_separately() {
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
                model_id: Some("claude-3-opus".to_string()),
                ..Default::default()
            },
        ),
    ];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r1 = index.match_request(&features("gpt-4o", "hi"));
    assert_eq!(index.get(r1.unwrap()).name, "gpt4-scene");

    let r2 = index.match_request(&features("claude-3-opus", "hi"));
    assert_eq!(index.get(r2.unwrap()).name, "claude-scene");

    let r3 = index.match_request(&features("unknown-model", "hi"));
    assert!(r3.is_none());
}

// ------------------------------------------------------------------
// Dual-dimension indexing: cross-protocol same-model isolation
// ------------------------------------------------------------------

#[test]
fn test_cross_protocol_same_model_routing() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r = index.match_request(&features_with_protocol(
        "gpt-4o",
        "hi",
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4");

    let r = index.match_request(&features_with_protocol(
        "gpt-4o",
        "hi",
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4");
}

#[test]
fn test_cross_protocol_fallback_matches_both_protocols() {
    let scenarios = vec![fallback("fallback")];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r = index.match_request(&features_with_protocol(
        "any-model",
        "hi",
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");

    let r = index.match_request(&features_with_protocol(
        "any-model",
        "hi",
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

#[test]
fn test_cross_protocol_model_specific_and_fallback_coexist() {
    // Model-specific + fallback = legal (fallback is the zero-match兜底).
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
    let result = MatcherIndex::build(scenarios);
    assert!(result.is_ok());
    let index = result.unwrap();
    // gpt-4o request hits specific
    let r = index.match_request(&features_with_protocol(
        "gpt-4o",
        "hi",
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "gpt4-scene");
    // non-matching model falls back
    let r = index.match_request(&features_with_protocol(
        "claude-3",
        "hi",
        ProtocolKind::OpenAi,
    ));
    assert_eq!(index.get(r.unwrap()).name, "fallback");
}

#[test]
fn test_cross_protocol_no_leakage_between_models() {
    let scenarios = vec![specific(
        "claude-scene",
        MatchCondition {
            model_id: Some("claude-3".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r = index.match_request(&features_with_protocol(
        "gpt-4o",
        "hi",
        ProtocolKind::OpenAi,
    ));
    assert!(r.is_none());

    let r = index.match_request(&features_with_protocol(
        "claude-3",
        "hi",
        ProtocolKind::Anthropic,
    ));
    assert_eq!(index.get(r.unwrap()).name, "claude-scene");
}

// ------------------------------------------------------------------
// match_scenario convenience function tests
// ------------------------------------------------------------------

#[test]
fn test_convenience_match_scenario_works() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    )];
    let result = match_scenario(&features("gpt-4o", "hi"), &scenarios).unwrap();
    assert_eq!(result.unwrap().name, "gpt4");
}

#[test]
fn test_convenience_match_scenario_no_match() {
    let scenarios = vec![specific(
        "gpt4",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            ..Default::default()
        },
    )];
    let result = match_scenario(&features("claude-3", "hi"), &scenarios).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_convenience_match_scenario_fallback() {
    let scenarios = vec![fallback("fallback")];
    let result = match_scenario(&features("any", "hi"), &scenarios).unwrap();
    assert_eq!(result.unwrap().name, "fallback");
}

// ------------------------------------------------------------------
// Additional edge case tests
// ------------------------------------------------------------------

#[test]
fn test_index_empty_messages_with_message_contains_no_match() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_full(
        "gpt-4o",
        vec![],
        vec![],
        ProtocolKind::OpenAi,
    ));
    assert!(result.is_none());
}

#[test]
fn test_index_tool_name_only_condition() {
    let scenarios = vec![specific(
        "code",
        MatchCondition {
            tool_name: Some("code_exec".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let r = index.match_request(&features_with_tools(
        "any-model",
        vec!["hi"],
        vec!["code_exec"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "code");

    assert!(index.match_request(&features("any-model", "hi")).is_none());
}

#[test]
fn test_index_two_scenarios_different_models_no_conflict() {
    let scenarios = vec![
        specific(
            "gpt4",
            MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            },
        ),
        specific(
            "claude",
            MatchCondition {
                model_id: Some("claude-3".to_string()),
                ..Default::default()
            },
        ),
    ];
    let index = MatcherIndex::build(scenarios).unwrap();
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "gpt-4o",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi
                    ))
                    .unwrap()
            )
            .name,
        "gpt4"
    );
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "claude-3",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi
                    ))
                    .unwrap()
            )
            .name,
        "claude"
    );
    assert!(index
        .match_request(&features_full(
            "gemini",
            vec![],
            vec![],
            ProtocolKind::OpenAi
        ))
        .is_none());
}

#[test]
fn test_index_partial_condition_match_fails() {
    let scenarios = vec![specific(
        "search",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            tool_name: Some("search".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    assert!(index.match_request(&features("gpt-4o", "hi")).is_none());
    assert!(index
        .match_request(&features_with_tools("claude-3", vec!["hi"], vec!["search"]))
        .is_none());
    let r = index.match_request(&features_with_tools("gpt-4o", vec!["hi"], vec!["search"]));
    assert_eq!(index.get(r.unwrap()).name, "search");
}

#[test]
fn test_index_message_contains_substring_matching() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("2+2".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    assert!(index
        .match_request(&features("gpt-4o", "what is 2+2?"))
        .is_some());
    assert!(index.match_request(&features("gpt-4o", "2+2")).is_some());
    assert!(index
        .match_request(&features("gpt-4o", "what is 3+3?"))
        .is_none());
}

#[test]
fn test_index_multiple_fallback_scenarios_different_models() {
    let scenarios = vec![
        ScenarioDeclaration {
            name: "gpt-fallback".to_string(),
            match_: Some(MatchCondition {
                model_id: Some("gpt-4o".to_string()),
                ..Default::default()
            }),
            turns: vec![text_turn("ok")],
            models: None,
        },
        ScenarioDeclaration {
            name: "claude-fallback".to_string(),
            match_: Some(MatchCondition {
                model_id: Some("claude-3".to_string()),
                ..Default::default()
            }),
            turns: vec![text_turn("ok")],
            models: None,
        },
    ];
    let index = MatcherIndex::build(scenarios).unwrap();
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "gpt-4o",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi
                    ))
                    .unwrap()
            )
            .name,
        "gpt-fallback"
    );
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "claude-3",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi
                    ))
                    .unwrap()
            )
            .name,
        "claude-fallback"
    );
}

#[test]
fn test_index_no_match_condition_matches_all_models() {
    let scenarios = vec![ScenarioDeclaration {
        name: "catch-all".to_string(),
        match_: None,
        turns: vec![text_turn("ok")],
        models: None,
    }];
    let index = MatcherIndex::build(scenarios).unwrap();
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "gpt-4o",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi
                    ))
                    .unwrap()
            )
            .name,
        "catch-all"
    );
    assert_eq!(
        index
            .get(
                index
                    .match_request(&features_full(
                        "any-model",
                        vec![],
                        vec![],
                        ProtocolKind::OpenAi,
                    ))
                    .unwrap()
            )
            .name,
        "catch-all"
    );
}

#[test]
fn test_index_message_in_second_message_matches() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_multi(
        "gpt-4o",
        vec![("user", "first message"), ("user", "please calculate this")],
    ));
    assert!(result.is_some());
}

#[test]
fn test_index_message_in_last_message_matches() {
    let scenarios = vec![specific(
        "math",
        MatchCondition {
            message_contains: Some("calculate".to_string()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();
    let result = index.match_request(&features_multi(
        "gpt-4o",
        vec![
            ("user", "first"),
            ("user", "second"),
            ("user", "please calculate"),
        ],
    ));
    assert!(result.is_some());
}
