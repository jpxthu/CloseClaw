//! Unit tests for workflow definition validation rules.

use crate::definition::{JumpQuestion, Step, Transition, Workflow};
use crate::definition_validator::validate_definition;

// -----------------------------------------------------------------------
// Helper: build a valid minimal workflow
// -----------------------------------------------------------------------

fn make_valid_workflow() -> Workflow {
    Workflow {
        id: "test".into(),
        name: "Test".into(),
        description: "Test workflow".into(),
        version: None,
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "Step".into(),
            allow_blocked: None,
            goal: "Do something".into(),
            verify: vec!["Verified".into()],
            jump: vec![],
            transitions: vec![Transition {
                when: None,
                action: "complete".into(),
                target_step: None,
            }],
        }],
    }
}

// -----------------------------------------------------------------------
// Rule 1: steps must not be empty
// -----------------------------------------------------------------------

#[test]
fn test_empty_steps_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps = vec![];
    let err = validate_definition(&wf).unwrap_err();
    assert!(
        format!("{err}").contains("steps must not be empty"),
        "unexpected error: {err}"
    );
}

// -----------------------------------------------------------------------
// Rule 2: step id continuity
// -----------------------------------------------------------------------

#[test]
fn test_valid_step_ids() {
    let wf = make_valid_workflow();
    assert!(validate_definition(&wf).is_ok());
}

#[test]
fn test_step_id_not_matching_index_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].id = 1;
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("expected id 0, got 1"));
}

#[test]
fn test_step_id_gap_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps.push(Step {
        id: 5,
        name: "Second".into(),
        allow_blocked: None,
        goal: "Second step".into(),
        verify: vec!["Check".into()],
        jump: vec![],
        transitions: vec![Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        }],
    });
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("expected id 1, got 5"));
}

#[test]
fn test_duplicate_step_id_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps.push(Step {
        id: 0,
        name: "Second".into(),
        allow_blocked: None,
        goal: "Second step".into(),
        verify: vec!["Check".into()],
        jump: vec![],
        transitions: vec![Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        }],
    });
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("expected id 1, got 0"));
}

// -----------------------------------------------------------------------
// Rule 3: name/goal non-empty
// -----------------------------------------------------------------------

#[test]
fn test_empty_name_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].name = "".into();
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("name must not be empty"));
}

#[test]
fn test_whitespace_only_name_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].name = "   \t  ".into();
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("name must not be empty"));
}

#[test]
fn test_empty_goal_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].goal = "".into();
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("goal must not be empty"));
}

// -----------------------------------------------------------------------
// Rule 4: verify non-empty
// -----------------------------------------------------------------------

#[test]
fn test_empty_verify_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].verify = vec![];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("verify checklist must not be empty"));
}

#[test]
fn test_blank_verify_item_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].verify = vec!["Check".into(), "  ".into()];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("verify item 1 must not be blank"));
}

// -----------------------------------------------------------------------
// Rule 5: transitions structure
// -----------------------------------------------------------------------

#[test]
fn test_empty_transitions_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].transitions = vec![];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("transitions must not be empty"));
}

#[test]
fn test_last_transition_must_be_default() {
    let mut wf = make_valid_workflow();
    wf.steps[0].transitions = vec![Transition {
        when: Some(serde_yaml::from_str("go: true").unwrap()),
        action: "complete".into(),
        target_step: None,
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("last transition must be the default"));
}

#[test]
fn test_non_last_transition_without_when_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].transitions = vec![
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("transition 0 must have 'when' condition"));
}

// -----------------------------------------------------------------------
// Rule 6: no duplicate when conditions
// -----------------------------------------------------------------------

#[test]
fn test_duplicate_when_conditions_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("duplicate 'when' condition"));
}

// -----------------------------------------------------------------------
// Rule 7: target validation
// -----------------------------------------------------------------------

#[test]
fn test_goto_target_missing_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: None,
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("target_step is invalid or out of range"));
}

#[test]
fn test_goto_target_out_of_range_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: Some(99),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("target_step is invalid or out of range"));
}

#[test]
fn test_reexecute_target_out_of_range_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "reexecute".into(),
            target_step: Some(99),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("target_step is invalid or out of range"));
}

#[test]
fn test_complete_action_no_target_required() {
    let wf = make_valid_workflow();
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Rule 8: when mapping validation
// -----------------------------------------------------------------------

#[test]
fn test_when_not_mapping_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::Value::Bool(true)),
            action: "complete".into(),
            target_step: None,
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("'when' must be a mapping"));
}

#[test]
fn test_when_key_not_matching_jump_id_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("nonexistent: true").unwrap()),
            action: "complete".into(),
            target_step: None,
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("does not match any jump id"));
}

#[test]
fn test_valid_when_references_jump_id() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Rule 9: expected_value validation
// -----------------------------------------------------------------------

#[test]
fn test_boolean_when_value_not_bool_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: yes_str").unwrap()),
            action: "complete".into(),
            target_step: None,
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("expected_value is not a bool"));
}

#[test]
fn test_boolean_when_value_bool_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "go".into(),
        prompt: "Go?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("go: true").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    assert!(validate_definition(&wf).is_ok());
}

#[test]
fn test_enum_when_value_not_in_options_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "strategy".into(),
        prompt: "Which?".into(),
        question_type: "enum".into(),
        options: vec!["fast".into(), "slow".into()],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("strategy: fast").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: Some(serde_yaml::from_str("strategy: unknown").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("is not in enum options"));
}

#[test]
fn test_enum_when_value_in_options_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "strategy".into(),
        prompt: "Which?".into(),
        question_type: "enum".into(),
        options: vec!["fast".into(), "slow".into()],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("strategy: fast").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: Some(serde_yaml::from_str("strategy: slow").unwrap()),
            action: "goto".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Rule 10: jump id uniqueness
// -----------------------------------------------------------------------

#[test]
fn test_duplicate_jump_id_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![
        JumpQuestion {
            id: "go".into(),
            prompt: "Go?".into(),
            question_type: "boolean".into(),
            options: vec![],
            option_labels: vec![],
        },
        JumpQuestion {
            id: "go".into(),
            prompt: "Go again?".into(),
            question_type: "boolean".into(),
            options: vec![],
            option_labels: vec![],
        },
    ];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("duplicate jump id 'go'"));
}

#[test]
fn test_unique_jump_ids_pass() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![
        JumpQuestion {
            id: "go".into(),
            prompt: "Go?".into(),
            question_type: "boolean".into(),
            options: vec![],
            option_labels: vec![],
        },
        JumpQuestion {
            id: "stop".into(),
            prompt: "Stop?".into(),
            question_type: "boolean".into(),
            options: vec![],
            option_labels: vec![],
        },
    ];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Rule 11: question_type validation
// -----------------------------------------------------------------------

#[test]
fn test_invalid_question_type_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "free_text".into(),
        options: vec![],
        option_labels: vec![],
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("unsupported question_type 'free_text'"));
}

#[test]
fn test_boolean_question_type_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    assert!(validate_definition(&wf).is_ok());
}

#[test]
fn test_enum_question_type_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec!["a".into(), "b".into()],
        option_labels: vec![],
    }];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Rule 12: enum options validation
// -----------------------------------------------------------------------

#[test]
fn test_enum_empty_options_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec![],
        option_labels: vec![],
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("enum options must not be empty"));
}

#[test]
fn test_enum_duplicate_options_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec!["a".into(), "a".into()],
        option_labels: vec![],
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("duplicate option 'a'"));
}

#[test]
fn test_enum_27_options_rejected() {
    let mut wf = make_valid_workflow();
    let options: Vec<String> = (0..27).map(|i| format!("opt_{i}")).collect();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options,
        option_labels: vec![],
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("must not exceed 26"));
}

#[test]
fn test_enum_26_options_passes() {
    let mut wf = make_valid_workflow();
    let options: Vec<String> = (0..26).map(|i| format!("opt_{i}")).collect();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options,
        option_labels: vec![],
    }];
    assert!(validate_definition(&wf).is_ok());
}

#[test]
fn test_enum_option_labels_length_mismatch_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec!["a".into(), "b".into()],
        option_labels: vec!["Label A".into()],
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("option_labels length"));
}

#[test]
fn test_enum_option_labels_matching_length_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec!["a".into(), "b".into()],
        option_labels: vec!["Label A".into(), "Label B".into()],
    }];
    assert!(validate_definition(&wf).is_ok());
}

#[test]
fn test_enum_option_labels_empty_passes() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "enum".into(),
        options: vec!["a".into(), "b".into()],
        option_labels: vec![],
    }];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Multi-step valid workflow
// -----------------------------------------------------------------------

#[test]
fn test_valid_multi_step_workflow() {
    let wf = Workflow {
        id: "multi".into(),
        name: "Multi".into(),
        description: "Multi step".into(),
        version: None,
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![
            Step {
                id: 0,
                name: "First".into(),
                allow_blocked: None,
                goal: "Step one".into(),
                verify: vec!["Done".into()],
                jump: vec![JumpQuestion {
                    id: "go_next".into(),
                    prompt: "Go?".into(),
                    question_type: "boolean".into(),
                    options: vec![],
                    option_labels: vec![],
                }],
                transitions: vec![
                    Transition {
                        when: Some(serde_yaml::from_str("go_next: true").unwrap()),
                        action: "goto".into(),
                        target_step: Some(1),
                    },
                    Transition {
                        when: None,
                        action: "complete".into(),
                        target_step: None,
                    },
                ],
            },
            Step {
                id: 1,
                name: "Second".into(),
                allow_blocked: None,
                goal: "Step two".into(),
                verify: vec!["Verified".into()],
                jump: vec![],
                transitions: vec![Transition {
                    when: None,
                    action: "complete".into(),
                    target_step: None,
                }],
            },
        ],
    };
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Reexecute valid target
// -----------------------------------------------------------------------

#[test]
fn test_reexecute_valid_target() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "retry".into(),
        prompt: "Retry?".into(),
        question_type: "boolean".into(),
        options: vec![],
        option_labels: vec![],
    }];
    wf.steps[0].transitions = vec![
        Transition {
            when: Some(serde_yaml::from_str("retry: true").unwrap()),
            action: "reexecute".into(),
            target_step: Some(0),
        },
        Transition {
            when: None,
            action: "complete".into(),
            target_step: None,
        },
    ];
    assert!(validate_definition(&wf).is_ok());
}

// -----------------------------------------------------------------------
// Unknown action rejected
// -----------------------------------------------------------------------

#[test]
fn test_unknown_action_rejected() {
    let mut wf = make_valid_workflow();
    wf.steps[0].transitions = vec![Transition {
        when: None,
        action: "invalid_action".into(),
        target_step: None,
    }];
    let err = validate_definition(&wf).unwrap_err();
    assert!(format!("{err}").contains("unknown action 'invalid_action'"));
}

// -----------------------------------------------------------------------
// Boolean jump with options (boolean type ignores options)
// -----------------------------------------------------------------------

#[test]
fn test_boolean_jump_options_ignored() {
    let mut wf = make_valid_workflow();
    wf.steps[0].jump = vec![JumpQuestion {
        id: "q".into(),
        prompt: "Question?".into(),
        question_type: "boolean".into(),
        options: vec!["anything".into()],
        option_labels: vec![],
    }];
    assert!(validate_definition(&wf).is_ok());
}
