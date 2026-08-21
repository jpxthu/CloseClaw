//! Comprehensive conflict detection boundary tests.
//!
//! Covers every dimension of the pairwise conflict detection logic:
//! field equality, substring containment, key-value compatibility,
//! fallback interactions, and the `from_dir` startup failure path.

use super::conflict::{detect_conflicts, ConflictReport};
use super::MatcherIndex;
use crate::scenario::types::{MatchCondition, ScenarioDeclaration};
use crate::scenario::types::{ResponseShape, TextResponse, TurnResponse};
use crate::types::ProtocolKind;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn text_turn() -> TurnResponse {
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

fn fallback(name: &str) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: None,
        turns: vec![text_turn()],
        models: None,
    }
}

fn conditional(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: Some(condition),
        turns: vec![text_turn()],
        models: None,
    }
}

fn feat(model: &str, msg: &str, protocol: ProtocolKind) -> crate::types::RequestFeatures {
    crate::types::RequestFeatures {
        model: model.to_string(),
        stream: false,
        max_tokens: None,
        temperature: None,
        messages: vec![crate::scenario::types::MessageEntry {
            role: "user".to_string(),
            content: msg.to_string(),
        }],
        tools: vec![],
        protocol,
    }
}

// ===================================================================
// All fields missing (MatchCondition::default()) — zero constraints
// ===================================================================

#[test]
fn all_fields_missing_matches_everything() {
    let a = conditional(
        "a",
        MatchCondition {
            model_id: None,
            message_contains: None,
            tool_name: None,
            request_params: None,
            ..Default::default()
        },
    );
    let b = conditional(
        "b",
        MatchCondition {
            model_id: None,
            message_contains: None,
            tool_name: None,
            request_params: None,
            ..Default::default()
        },
    );
    let conflicts = detect_conflicts(&[a, b]);
    assert_eq!(conflicts.len(), 1);
}

#[test]
fn all_fields_missing_one_empty_params() {
    let a = conditional(
        "a",
        MatchCondition {
            request_params: Some(serde_json::from_str("{}").unwrap()),
            ..Default::default()
        },
    );
    let b = conditional(
        "b",
        MatchCondition {
            request_params: Some(serde_json::from_str("{}").unwrap()),
            ..Default::default()
        },
    );
    let conflicts = detect_conflicts(&[a, b]);
    assert_eq!(conflicts.len(), 1);
}

// ===================================================================
// model_id: equal / unequal / one missing
// ===================================================================

#[test]
fn model_id_equal_conflicts() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn model_id_unequal_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: Some("claude-3".into()),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn model_id_one_missing_compatible() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: None,
                ..Default::default()
            },
        ),
    ];
    // Both fields default to None except model_id, so all fields compatible.
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

// ===================================================================
// message_contains: equal / substring / disjoint
// ===================================================================

#[test]
fn msg_equal_conflicts() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                message_contains: Some("hello".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                message_contains: Some("hello".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn msg_substring_conflicts() {
    // "calc" is substring of "calculate"
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                message_contains: Some("calc".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                message_contains: Some("calculate".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn msg_substring_reversed_conflicts() {
    // "calculate" contains "calc" — same direction reversed
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                message_contains: Some("calculate".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                message_contains: Some("calc".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn msg_disjoint_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                message_contains: Some("foo".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                message_contains: Some("bar".into()),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn msg_one_missing_compatible() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                message_contains: Some("hello".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                message_contains: None,
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

// ===================================================================
// tool_name: equal / unequal / one missing
// ===================================================================

#[test]
fn tool_equal_conflicts() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                tool_name: Some("search".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                tool_name: Some("search".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn tool_unequal_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                tool_name: Some("search".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                tool_name: Some("code_exec".into()),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn tool_one_missing_compatible() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                tool_name: Some("search".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                tool_name: None,
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

// ===================================================================
// request_params: value equal / unequal / key missing
// ===================================================================

#[test]
fn params_value_equal_conflicts() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn params_value_unequal_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.5))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.9))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn params_one_key_missing_conflicts() {
    // A requires stream=true, B requires max_tokens=1024.
    // A request with stream=true AND max_tokens=1024 matches both.
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("max_tokens".into(), serde_json::json!(1024))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn params_one_side_has_no_params_compatible() {
    // B has no request_params constraint at all → compatible.
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: None,
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn params_temperature_f32_precision_equal_conflicts() {
    // 0.1 + 0.2 != 0.3 in f64, but f32(0.1+0.2) == f32(0.3)
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.1_f64 + 0.2_f64))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.3_f64))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    // f32(0.1+0.2) == f32(0.3) → conflict
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn params_temperature_f32_precision_different_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.5))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("temperature".into(), serde_json::json!(0.7))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn params_bool_vs_string_different_no_conflict() {
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!(true))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                request_params: Some(
                    vec![("stream".into(), serde_json::json!("yes"))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

// ===================================================================
// Fallback × fallback / fallback × conditional
// ===================================================================

#[test]
fn two_fallbacks_conflict() {
    let scenarios = vec![fallback("a"), fallback("b")];
    let conflicts = detect_conflicts(&scenarios);
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].reason.contains("fallback"));
}

#[test]
fn fallback_and_conditional_conflict() {
    let scenarios = vec![
        fallback("fb"),
        conditional(
            "specific",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
    ];
    let conflicts = detect_conflicts(&scenarios);
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].reason.contains("fallback"));
}

// ===================================================================
// Cross-protocol: same model_id in declarations → conflict at detection
// level (protocol isolation is enforced by index bucketing)
// ===================================================================

#[test]
fn same_model_conditional_conflict_at_detection_level() {
    // Both have model_id="gpt-4o" with same message_contains → conflict.
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                message_contains: Some("hello".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                message_contains: Some("hello".into()),
                ..Default::default()
            },
        ),
    ];
    assert_eq!(detect_conflicts(&scenarios).len(), 1);
}

#[test]
fn same_model_different_message_no_conflict_at_detection() {
    // model_id equal but message_contains disjoint → conditions mutually
    // exclusive at the field level → no conflict.
    let scenarios = vec![
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                message_contains: Some("foo".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                message_contains: Some("bar".into()),
                ..Default::default()
            },
        ),
    ];
    assert!(detect_conflicts(&scenarios).is_empty());
}

// ===================================================================
// from_dir startup failure path
// ===================================================================

#[test]
fn from_dir_conflicting_scenarios_fails_at_startup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let conflicting_json = serde_json::json!({
        "scenarios": [
            {
                "name": "scene-a",
                "match_": { "model_id": "gpt-4o" },
                "turns": [{ "response": { "type": "text", "content": "a" } }]
            },
            {
                "name": "scene-b",
                "match_": { "model_id": "gpt-4o" },
                "turns": [{ "response": { "type": "text", "content": "b" } }]
            }
        ]
    });
    std::fs::write(
        tmp.path().join("conflict.json"),
        serde_json::to_string(&conflicting_json).unwrap(),
    )
    .unwrap();

    let result = crate::scenario::ScenarioEngine::from_dir(tmp.path());
    assert!(
        result.is_err(),
        "from_dir must fail for conflicting scenarios"
    );
    let err_msg = match result {
        Ok(_) => unreachable!(),
        Err(e) => format!("{}", e),
    };
    assert!(
        err_msg.contains("conflict"),
        "error message should mention conflict: {}",
        err_msg
    );
}

#[test]
fn from_dir_fallback_and_conditional_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let json = serde_json::json!({
        "scenarios": [
            {
                "name": "fallback",
                "turns": [{ "response": { "type": "text", "content": "fb" } }]
            },
            {
                "name": "specific",
                "match_": { "model_id": "gpt-4o" },
                "turns": [{ "response": { "type": "text", "content": "sp" } }]
            }
        ]
    });
    std::fs::write(
        tmp.path().join("bad.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    let result = crate::scenario::ScenarioEngine::from_dir(tmp.path());
    assert!(result.is_err());
    let err_msg = match result {
        Ok(_) => unreachable!(),
        Err(e) => format!("{}", e),
    };
    assert!(err_msg.contains("conflict"));
}

#[test]
fn from_dir_no_conflict_succeeds() {
    let tmp = tempfile::TempDir::new().unwrap();
    let json = serde_json::json!({
        "scenarios": [
            {
                "name": "scene-a",
                "match_": { "model_id": "gpt-4o" },
                "turns": [{ "response": { "type": "text", "content": "a" } }]
            },
            {
                "name": "scene-b",
                "match_": { "model_id": "claude-3" },
                "turns": [{ "response": { "type": "text", "content": "b" } }]
            }
        ]
    });
    std::fs::write(
        tmp.path().join("ok.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();

    let result = crate::scenario::ScenarioEngine::from_dir(tmp.path());
    assert!(
        result.is_ok(),
        "from_dir should succeed for non-conflicting scenarios"
    );
}

// ===================================================================
// MatcherIndex build-time conflict propagation
// ===================================================================

#[test]
fn matcher_index_build_conflict_error_carry_names() {
    let scenarios = vec![
        conditional(
            "scene-x",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
        conditional(
            "scene-y",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
    ];
    let result = MatcherIndex::build(scenarios);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let names: Vec<&str> = err
        .conflicts
        .iter()
        .flat_map(|c| [c.scenario_a.as_str(), c.scenario_b.as_str()])
        .collect();
    assert!(names.contains(&"scene-x"));
    assert!(names.contains(&"scene-y"));
}

// ===================================================================
// Multiple conflicts detected
// ===================================================================

#[test]
fn three_way_conflict_all_pairs() {
    let scenarios = vec![
        fallback("fb"),
        conditional(
            "a",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
        conditional(
            "b",
            MatchCondition {
                model_id: Some("gpt-4o".into()),
                ..Default::default()
            },
        ),
    ];
    // fb × a, fb × b, a × b = 3 conflicts
    assert_eq!(detect_conflicts(&scenarios).len(), 3);
}

// ===================================================================
// ConflictReport Display
// ===================================================================

#[test]
fn conflict_report_display_contains_all_fields() {
    let report = ConflictReport {
        scenario_a: "alpha".into(),
        scenario_b: "beta".into(),
        reason: "test reason".into(),
    };
    let s = format!("{}", report);
    assert!(s.contains("alpha"));
    assert!(s.contains("beta"));
    assert!(s.contains("test reason"));
}

// ===================================================================
// Dual-dimension index routing via MatcherIndex (from_dir integration)
// ===================================================================

#[test]
fn dual_dimension_same_model_different_protocol_independent_hit() {
    let scenarios = vec![conditional(
        "openai-scene",
        MatchCondition {
            model_id: Some("gpt-4o".into()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r1 = index.match_request(&feat("gpt-4o", "hi", ProtocolKind::OpenAi));
    assert_eq!(index.get(r1.unwrap()).name, "openai-scene");

    // Same model, Anthropic protocol → still hits (scenario is not
    // protocol-constrained in its declaration, so it's in both buckets).
    let r2 = index.match_request(&feat("gpt-4o", "hi", ProtocolKind::Anthropic));
    assert_eq!(index.get(r2.unwrap()).name, "openai-scene");
}

#[test]
fn dual_dimension_any_model_bucket_isolated_by_protocol() {
    let scenarios = vec![fallback("fb")];
    let index = MatcherIndex::build(scenarios).unwrap();

    let r1 = index.match_request(&feat("any-model", "hi", ProtocolKind::OpenAi));
    assert_eq!(index.get(r1.unwrap()).name, "fb");

    let r2 = index.match_request(&feat("any-model", "hi", ProtocolKind::Anthropic));
    assert_eq!(index.get(r2.unwrap()).name, "fb");
}

// ===================================================================
// Empty / boundary conditions
// ===================================================================

#[test]
fn empty_scenarios_no_conflicts() {
    assert!(detect_conflicts(&[]).is_empty());
}

#[test]
fn single_scenario_no_conflicts() {
    let scenarios = vec![conditional(
        "only",
        MatchCondition {
            model_id: Some("gpt-4o".into()),
            ..Default::default()
        },
    )];
    assert!(detect_conflicts(&scenarios).is_empty());
}

#[test]
fn single_fallback_no_conflicts() {
    let scenarios = vec![fallback("only")];
    assert!(detect_conflicts(&scenarios).is_empty());
}
