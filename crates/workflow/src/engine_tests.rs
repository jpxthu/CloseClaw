//! Unit tests for workflow engine: transition evaluation, state machine,
//! and end-to-end lifecycle.

use std::collections::HashMap;

use crate::definition::Transition;
use crate::engine::evaluate_transitions;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_transition(
    when: Option<serde_yaml::Value>,
    action: &str,
    target: Option<usize>,
) -> Transition {
    Transition {
        when,
        action: action.to_string(),
        target_step: target,
    }
}

fn when_map(entries: &[(&str, serde_yaml::Value)]) -> serde_yaml::Value {
    let mut map = serde_yaml::Mapping::new();
    for (k, v) in entries {
        map.insert(serde_yaml::Value::String(k.to_string()), v.clone());
    }
    serde_yaml::Value::Mapping(map)
}

// ---------------------------------------------------------------------------

#[test]
fn test_boolean_true_matches() {
    let transitions = vec![make_transition(
        Some(when_map(&[("needs_pr", serde_yaml::Value::Bool(true))])),
        "goto",
        Some(1),
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(true));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(target, Some(1));
}

#[test]
fn test_boolean_false_matches() {
    let transitions = vec![make_transition(
        Some(when_map(&[("needs_pr", serde_yaml::Value::Bool(false))])),
        "complete",
        None,
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(false));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, _) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Complete);
}

#[test]
fn test_boolean_wrong_value_no_match() {
    let transitions = vec![make_transition(
        Some(when_map(&[("needs_pr", serde_yaml::Value::Bool(true))])),
        "goto",
        Some(1),
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(false));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// Enum condition matching (string comparison)
// ---------------------------------------------------------------------------

#[test]
fn test_enum_single_option_matches() {
    let transitions = vec![make_transition(
        Some(when_map(&[(
            "path",
            serde_yaml::Value::String("fast".into()),
        )])),
        "goto",
        Some(0),
    )];
    let mut answers = HashMap::new();
    answers.insert("path".to_string(), serde_yaml::Value::String("fast".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(0));
    assert_eq!(target, Some(0));
}

#[test]
fn test_enum_wrong_option_no_match() {
    let transitions = vec![make_transition(
        Some(when_map(&[(
            "path",
            serde_yaml::Value::String("fast".into()),
        )])),
        "goto",
        Some(0),
    )];
    let mut answers = HashMap::new();
    answers.insert("path".to_string(), serde_yaml::Value::String("slow".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// AND logic: multiple conditions in a single when
// ---------------------------------------------------------------------------

#[test]
fn test_multi_condition_all_match() {
    let transitions = vec![make_transition(
        Some(when_map(&[
            ("needs_pr", serde_yaml::Value::Bool(true)),
            ("path", serde_yaml::Value::String("fast".into())),
        ])),
        "goto",
        Some(2),
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(true));
    answers.insert("path".to_string(), serde_yaml::Value::String("fast".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(2));
    assert_eq!(target, Some(2));
}

#[test]
fn test_multi_condition_partial_match_fails() {
    let transitions = vec![make_transition(
        Some(when_map(&[
            ("needs_pr", serde_yaml::Value::Bool(true)),
            ("path", serde_yaml::Value::String("fast".into())),
        ])),
        "goto",
        Some(2),
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(true));
    answers.insert("path".to_string(), serde_yaml::Value::String("slow".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

#[test]
fn test_multi_condition_missing_answer_fails() {
    let transitions = vec![make_transition(
        Some(when_map(&[
            ("needs_pr", serde_yaml::Value::Bool(true)),
            ("path", serde_yaml::Value::String("fast".into())),
        ])),
        "goto",
        Some(2),
    )];
    let mut answers = HashMap::new();
    answers.insert("needs_pr".to_string(), serde_yaml::Value::Bool(true));
    // "path" answer is missing

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// Transition ordering: first match wins
// ---------------------------------------------------------------------------

#[test]
fn test_first_matching_transition_wins() {
    let transitions = vec![
        make_transition(
            Some(when_map(&[("q", serde_yaml::Value::String("a".into()))])),
            "goto",
            Some(1),
        ),
        make_transition(
            Some(when_map(&[("q", serde_yaml::Value::String("a".into()))])),
            "goto",
            Some(2),
        ),
    ];
    let mut answers = HashMap::new();
    answers.insert("q".to_string(), serde_yaml::Value::String("a".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(target, Some(1));
}

// ---------------------------------------------------------------------------
// Default fallback (transition without when)
// ---------------------------------------------------------------------------

#[test]
fn test_default_fallback_when_no_match() {
    let transitions = vec![
        make_transition(
            Some(when_map(&[("q", serde_yaml::Value::String("x".into()))])),
            "goto",
            Some(1),
        ),
        make_transition(None, "complete", None),
    ];
    let mut answers = HashMap::new();
    answers.insert("q".to_string(), serde_yaml::Value::String("y".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, _) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Complete);
}

#[test]
fn test_default_not_used_when_condition_matches() {
    let transitions = vec![
        make_transition(
            Some(when_map(&[("q", serde_yaml::Value::String("a".into()))])),
            "goto",
            Some(1),
        ),
        make_transition(None, "complete", None),
    ];
    let mut answers = HashMap::new();
    answers.insert("q".to_string(), serde_yaml::Value::String("a".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(target, Some(1));
}

// ---------------------------------------------------------------------------
// No match and no default → None
// ---------------------------------------------------------------------------

#[test]
fn test_no_match_no_default_returns_none() {
    let transitions = vec![make_transition(
        Some(when_map(&[("q", serde_yaml::Value::String("a".into()))])),
        "goto",
        Some(1),
    )];
    let mut answers = HashMap::new();
    answers.insert("q".to_string(), serde_yaml::Value::String("b".into()));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

#[test]
fn test_empty_transitions_returns_none() {
    let transitions: Vec<Transition> = vec![];
    let answers = HashMap::new();

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// Edge: default only (no when conditions at all)
// ---------------------------------------------------------------------------

#[test]
fn test_default_only_transition() {
    let transitions = vec![make_transition(None, "reexecute", Some(0))];
    let answers = HashMap::new();

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(target, Some(0));
}

// ---------------------------------------------------------------------------
// Edge: reexecute action
// ---------------------------------------------------------------------------

#[test]
fn test_reexecute_action() {
    let transitions = vec![make_transition(
        Some(when_map(&[("retry", serde_yaml::Value::Bool(true))])),
        "reexecute",
        Some(0),
    )];
    let mut answers = HashMap::new();
    answers.insert("retry".to_string(), serde_yaml::Value::Bool(true));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, target) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(target, Some(0));
}

// ---------------------------------------------------------------------------
// Edge: answers has extra keys not referenced in when
// ---------------------------------------------------------------------------

#[test]
fn test_extra_answers_ignored() {
    let transitions = vec![make_transition(
        Some(when_map(&[("q1", serde_yaml::Value::Bool(true))])),
        "goto",
        Some(1),
    )];
    let mut answers = HashMap::new();
    answers.insert("q1".to_string(), serde_yaml::Value::Bool(true));
    answers.insert("q2".to_string(), serde_yaml::Value::Bool(false));

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
}

// ---------------------------------------------------------------------------
// Edge: empty when mapping (should always match)
// ---------------------------------------------------------------------------

#[test]
fn test_empty_when_always_matches() {
    let transitions = vec![make_transition(
        Some(serde_yaml::Value::Mapping(serde_yaml::Mapping::new())),
        "complete",
        None,
    )];
    let answers = HashMap::new();

    let result = evaluate_transitions(&transitions, &answers);
    assert!(result.is_some());
    let (action, _) = result.unwrap();
    assert_eq!(action, crate::definition::JumpAction::Complete);
}
