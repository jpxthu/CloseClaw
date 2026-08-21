//! request_params matching tests.

use super::super::types::MessageEntry;
use super::*;
use crate::scenario::types::{MatchCondition, ResponseShape, TextResponse, TurnResponse};

/// Helper to build `RequestFeatures` with stream/max_tokens/temperature.
fn feat_with_params(
    model: &str,
    stream: bool,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    messages: Vec<&str>,
    tools: Vec<&str>,
) -> RequestFeatures {
    RequestFeatures {
        model: model.to_string(),
        stream,
        max_tokens,
        temperature,
        messages: messages
            .into_iter()
            .map(|c| MessageEntry {
                role: "user".to_string(),
                content: c.to_string(),
            })
            .collect(),
        tools: tools.into_iter().map(String::from).collect(),
    }
}

fn specific(name: &str, condition: MatchCondition) -> ScenarioDeclaration {
    ScenarioDeclaration {
        name: name.to_string(),
        match_: Some(condition),
        turns: vec![TurnResponse {
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
        }],
        models: None,
    }
}

#[test]
fn request_params_stream_true_matches() {
    let scenarios = vec![specific(
        "streaming",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"stream": true}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "streaming");
}

#[test]
fn request_params_stream_true_no_match_when_false() {
    let scenarios = vec![specific(
        "streaming",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"stream": true}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());
}

#[test]
fn request_params_stream_false_matches() {
    let scenarios = vec![specific(
        "no-stream",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"stream": false}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "no-stream");
}

#[test]
fn request_params_max_tokens_exact_match() {
    let scenarios = vec![specific(
        "max-tok",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"max_tokens": 1024}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        Some(1024),
        None,
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "max-tok");
}

#[test]
fn request_params_max_tokens_no_match_when_different() {
    let scenarios = vec![specific(
        "max-tok",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"max_tokens": 1024}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        Some(2048),
        None,
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());
}

#[test]
fn request_params_max_tokens_none_in_condition_matches_any() {
    let scenarios = vec![specific(
        "max-tok",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"max_tokens": 1024}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // max_tokens=None in request: features_to_json_value returns None =>
    // match returns true for unknown/missing key
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_some());
}

#[test]
fn request_params_temperature_exact_match() {
    let scenarios = vec![specific(
        "temp",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"temperature": 0.7}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        Some(0.7),
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "temp");
}

#[test]
fn request_params_temperature_no_match_when_different() {
    let scenarios = vec![specific(
        "temp",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"temperature": 0.7}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        Some(1.0),
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());
}

#[test]
fn request_params_temperature_none_in_condition_matches_any() {
    let scenarios = vec![specific(
        "temp",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{"temperature": 0.7}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // temperature=None in request: features_to_json_value returns None =>
    // match returns true for unknown/missing key
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_some());
}

#[test]
fn request_params_multi_param_all_must_match() {
    let scenarios = vec![specific(
        "multi",
        MatchCondition {
            request_params: Some(
                serde_json::from_str(r#"{"stream": true, "max_tokens": 512, "temperature": 0.5}"#)
                    .unwrap(),
            ),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // All three match
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(512),
        Some(0.5),
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "multi");

    // stream matches but max_tokens doesn't
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(1024),
        Some(0.5),
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());

    // All params match but stream doesn't
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        Some(512),
        Some(0.5),
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());
}

#[test]
fn request_params_none_does_not_affect_other_conditions() {
    let scenarios = vec![specific(
        "model-only",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            request_params: None,
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // request_params=None => always passes
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(1024),
        Some(0.9),
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "model-only");

    // Same with false stream and no other params
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "model-only");
}

#[test]
fn request_params_with_model_id_and_message_and_tool() {
    let scenarios = vec![specific(
        "combined",
        MatchCondition {
            model_id: Some("gpt-4o".to_string()),
            message_contains: Some("hello".to_string()),
            tool_name: Some("search".to_string()),
            request_params: Some(
                serde_json::from_str(r#"{"stream": true, "max_tokens": 256}"#).unwrap(),
            ),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);

    // All conditions satisfied
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(256),
        None,
        vec!["hello world"],
        vec!["search"],
    ));
    assert_eq!(index.get(r.unwrap()).name, "combined");

    // Model wrong
    let r = index.match_request(&feat_with_params(
        "claude-3",
        true,
        Some(256),
        None,
        vec!["hello world"],
        vec!["search"],
    ));
    assert!(r.is_none());

    // Message missing
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(256),
        None,
        vec!["goodbye"],
        vec!["search"],
    ));
    assert!(r.is_none());

    // Tool missing
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(256),
        None,
        vec!["hello"],
        vec![],
    ));
    assert!(r.is_none());

    // stream wrong
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        false,
        Some(256),
        None,
        vec!["hello"],
        vec!["search"],
    ));
    assert!(r.is_none());

    // max_tokens wrong
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(512),
        None,
        vec!["hello"],
        vec!["search"],
    ));
    assert!(r.is_none());
}

#[test]
fn request_params_unknown_key_ignored() {
    let scenarios = vec![specific(
        "unknown",
        MatchCondition {
            request_params: Some(
                serde_json::from_str(r#"{"unknown_field": "value", "stream": true}"#).unwrap(),
            ),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // Unknown key "unknown_field" is ignored; stream=true matches
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "unknown");
}

#[test]
fn request_params_empty_map_matches_all() {
    let scenarios = vec![specific(
        "empty-params",
        MatchCondition {
            request_params: Some(serde_json::from_str(r#"{}"#).unwrap()),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);
    // Empty request_params => all requests pass the params check
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        Some(999),
        Some(2.0),
        vec!["hi"],
        vec![],
    ));
    assert_eq!(index.get(r.unwrap()).name, "empty-params");
}

#[test]
fn request_params_partial_multi_param_failure() {
    let scenarios = vec![specific(
        "partial",
        MatchCondition {
            request_params: Some(
                serde_json::from_str(r#"{"stream": true, "temperature": 0.7}"#).unwrap(),
            ),
            ..Default::default()
        },
    )];
    let index = MatcherIndex::build(scenarios);

    // Only stream matches, temperature missing -> pass (None => unknown key => ignore)
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        None,
        None,
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_some());

    // Stream matches but temperature wrong
    let r = index.match_request(&feat_with_params(
        "gpt-4o",
        true,
        None,
        Some(1.0),
        vec!["hi"],
        vec![],
    ));
    assert!(r.is_none());
}
