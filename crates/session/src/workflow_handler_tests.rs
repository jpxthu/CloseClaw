//! Tests for workflow handler tool result processing.

use closeclaw_common::ContentBlock;
use closeclaw_workflow::definition::{JumpQuestion, Step, Workflow};
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};

use crate::workflow_handler::WorkflowHandler;
use crate::workflow_port::real_engine_port::test_port;

fn make_handler(run: WorkflowRun, wf: Workflow) -> WorkflowHandler {
    WorkflowHandler::new(
        serde_json::to_value(&run).unwrap(),
        serde_json::to_value(&wf).unwrap(),
        test_port(),
    )
}

/// Decode the handler's serialized run state into the typed form.
fn typed_state(handler: &WorkflowHandler) -> WorkflowRun {
    serde_json::from_value(handler.run_state().clone()).unwrap()
}

fn make_test_workflow() -> Workflow {
    Workflow {
        id: "test-wf".to_string(),
        name: "Test Workflow".to_string(),
        description: "A test workflow".to_string(),
        version: Some("0.1".to_string()),
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![
            Step {
                id: 0,
                name: "Step 0".to_string(),
                goal: "Do first thing".to_string(),
                verify: vec!["Check output".to_string()],
                jump: vec![],
                transitions: vec![],
                allow_blocked: Some(true),
            },
            Step {
                id: 1,
                name: "Step 1".to_string(),
                goal: "Do second thing".to_string(),
                verify: vec!["Check result".to_string()],
                jump: vec![],
                transitions: vec![],
                allow_blocked: Some(false),
            },
            Step {
                id: 2,
                name: "Step 2".to_string(),
                goal: "Do third thing".to_string(),
                verify: vec!["Check third".to_string()],
                jump: vec![],
                transitions: vec![],
                allow_blocked: None, // inherits from workflow
            },
        ],
    }
}

/// Workflow with allow_blocked=true at workflow level, no step overrides.
fn make_workflow_level_blocked_workflow() -> Workflow {
    Workflow {
        id: "wf-level-blocked".to_string(),
        name: "WF Level Blocked".to_string(),
        description: "Workflow with allow_blocked=true".to_string(),
        version: Some("0.1".to_string()),
        allow_blocked: true,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "Step 0".to_string(),
            goal: "Do thing".to_string(),
            verify: vec![],
            jump: vec![],
            transitions: vec![],
            allow_blocked: None, // inherits from workflow (true)
        }],
    }
}

fn make_enum_workflow() -> Workflow {
    Workflow {
        id: "enum-test".to_string(),
        name: "Enum Test".to_string(),
        description: "Workflow with enum questions".to_string(),
        version: Some("0.1".to_string()),
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "Decide".to_string(),
            goal: "Choose".to_string(),
            verify: vec![],
            jump: vec![
                JumpQuestion {
                    id: "strategy".to_string(),
                    prompt: "Which strategy?".to_string(),
                    question_type: "enum".to_string(),
                    options: vec!["fast".to_string(), "slow".to_string()],
                    option_labels: vec![],
                },
                JumpQuestion {
                    id: "mode".to_string(),
                    prompt: "Which mode?".to_string(),
                    question_type: "boolean".to_string(),
                    options: vec![],
                    option_labels: vec![],
                },
                JumpQuestion {
                    id: "empty_opts".to_string(),
                    prompt: "Empty options?".to_string(),
                    question_type: "enum".to_string(),
                    options: vec![],
                    option_labels: vec![],
                },
            ],
            transitions: vec![],
            allow_blocked: None,
        }],
    }
}

fn make_test_run() -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test Workflow".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: PendingVerify::default(),
        paused_reason: String::new(),
    }
}

#[test]
fn test_process_tool_result_start() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_start", "name": "Test Workflow"}"#;
    assert!(handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Executing);
}

#[test]
fn test_process_tool_result_verify_no_transitions() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_verify"}"#;
    // No jump questions in step 0 and no transitions → NoMatchingTransition error → returns false
    assert!(!handler.process_tool_result(content).0);
}

#[test]
fn test_process_tool_result_blocked_allowed() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Blocked);
    let notif = handler.take_notification();
    assert!(notif.is_some());
    let notif = notif.unwrap();
    assert_eq!(notif.workflow_name, "Test Workflow");
    assert_eq!(notif.current_step, 0);
    assert!(notif.reason.contains("need help"));
}

#[test]
fn test_process_tool_result_blocked_not_allowed() {
    let mut run = make_test_run();
    run.current_step = 1; // step 1 has allow_blocked = false
    let mut handler = make_handler(run, make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(!handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Executing);
}

#[test]
fn test_process_tool_result_unknown_action() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "unknown_action"}"#;
    assert!(!handler.process_tool_result(content).0);
}

#[test]
fn test_process_tool_result_invalid_json() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    assert!(!handler.process_tool_result("not json").0);
}

#[test]
fn test_process_content_blocks() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let blocks = vec![
        ContentBlock::Text("some text".to_string()),
        ContentBlock::ToolResult {
            tool_call_id: "call-1".to_string(),
            content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
        },
    ];
    assert!(handler.process_content_blocks(&blocks).0);
    assert_eq!(typed_state(&handler).phase, Phase::Blocked);
}

#[test]
fn test_process_content_blocks_no_workflow() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "call-1".to_string(),
        content: r#"{"action": "some_other_tool"}"#.to_string(),
    }];
    assert!(!handler.process_content_blocks(&blocks).0);
    assert_eq!(typed_state(&handler).phase, Phase::Executing);
}

#[test]
fn test_on_owner_resolve() {
    let mut run = make_test_run();
    run.phase = Phase::Blocked;
    run.pending_verify.count = 3;
    let mut handler = make_handler(run, make_test_workflow());
    handler.on_owner_resolve();
    let state = typed_state(&handler);
    assert_eq!(state.phase, Phase::Verifying);
    assert_eq!(state.pending_verify.count, 0);
}

#[test]
fn test_on_owner_terminate() {
    let mut run = make_test_run();
    run.phase = Phase::Blocked;
    let mut handler = make_handler(run, make_test_workflow());
    handler.on_owner_terminate();
    assert_eq!(typed_state(&handler).phase, Phase::Complete);
}

#[test]
fn test_is_blocked() {
    let mut run = make_test_run();
    let handler = make_handler(run.clone(), make_test_workflow());
    assert!(!handler.is_blocked());
    run.phase = Phase::Blocked;
    let handler = make_handler(run, make_test_workflow());
    assert!(handler.is_blocked());
}

#[test]
fn test_is_complete() {
    let mut run = make_test_run();
    let handler = make_handler(run.clone(), make_test_workflow());
    assert!(!handler.is_complete());
    run.phase = Phase::Complete;
    let handler = make_handler(run, make_test_workflow());
    assert!(handler.is_complete());
}

#[test]
fn test_on_goal_injected() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    handler.on_goal_injected();
    // step_data is cleared by on_goal_injected
    assert!(typed_state(&handler).step_data.is_null());
}

#[test]
fn test_notification_taken_only_once() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "test"}"#;
    handler.process_tool_result(content);
    assert!(handler.take_notification().is_some());
    assert!(handler.take_notification().is_none());
}

#[test]
fn test_blocked_persists_paused_reason() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(handler.process_tool_result(content).0);
    let state = typed_state(&handler);
    assert_eq!(state.phase, Phase::Blocked);
    assert_eq!(state.paused_reason, "need help");
}

#[test]
fn test_blocked_clears_paused_reason_on_not_allowed() {
    let mut run = make_test_run();
    run.current_step = 1; // step 1 has allow_blocked = false
    let mut handler = make_handler(run, make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "should not persist"}"#;
    assert!(!handler.process_tool_result(content).0);
    let state = typed_state(&handler);
    assert_eq!(state.phase, Phase::Executing);
    assert!(state.paused_reason.is_empty());
}

#[test]
fn test_on_verify_limit_exceeded() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    handler.on_verify_injected(3);
    assert_eq!(typed_state(&handler).pending_verify.count, 1);
    assert_eq!(typed_state(&handler).phase, Phase::Verifying);

    handler.on_verify_injected(3);
    assert_eq!(typed_state(&handler).pending_verify.count, 2);

    handler.on_verify_injected(3);
    assert_eq!(typed_state(&handler).pending_verify.count, 3);

    // 4th call exceeds limit of 3
    handler.on_verify_injected(3);
    assert_eq!(typed_state(&handler).phase, Phase::Blocked);
    assert!(handler.take_notification().is_some());
}

#[test]
fn test_on_verify_injected_within_limit() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    handler.on_verify_injected(5);
    assert_eq!(typed_state(&handler).pending_verify.count, 1);
    assert_eq!(typed_state(&handler).phase, Phase::Verifying);
    assert!(handler.take_notification().is_none());
}

// ── map_enum_letter_answers ──────────────────────────────────────

#[test]
fn test_enum_letter_a_maps_to_first_option() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    answers.insert("strategy".into(), serde_json::Value::String("A".into()));
    handler.map_enum_letter_answers(&mut answers);
    assert_eq!(
        answers["strategy"],
        serde_json::Value::String("fast".into())
    );
}

#[test]
fn test_enum_letter_b_maps_to_second_option() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    answers.insert("strategy".into(), serde_json::Value::String("B".into()));
    handler.map_enum_letter_answers(&mut answers);
    assert_eq!(
        answers["strategy"],
        serde_json::Value::String("slow".into())
    );
}

#[test]
fn test_enum_letter_c_out_of_range_not_mapped() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    answers.insert("strategy".into(), serde_json::Value::String("C".into()));
    handler.map_enum_letter_answers(&mut answers);
    // C (index 2) >= options.len() (2) → not mapped
    assert_eq!(answers["strategy"], serde_json::Value::String("C".into()));
}

#[test]
fn test_enum_non_enum_question_not_mapped() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    // "mode" has question_type = "boolean", not "enum"
    answers.insert("mode".into(), serde_json::Value::String("A".into()));
    handler.map_enum_letter_answers(&mut answers);
    assert_eq!(answers["mode"], serde_json::Value::String("A".into()));
}

#[test]
fn test_enum_lowercase_letter_not_mapped() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    answers.insert("strategy".into(), serde_json::Value::String("a".into()));
    handler.map_enum_letter_answers(&mut answers);
    assert_eq!(answers["strategy"], serde_json::Value::String("a".into()));
}

#[test]
fn test_enum_empty_options_not_mapped() {
    let handler = make_handler(make_test_run(), make_enum_workflow());
    let mut answers = serde_json::Map::new();
    // "empty_opts" has question_type = "enum" but options is empty
    answers.insert("empty_opts".into(), serde_json::Value::String("A".into()));
    handler.map_enum_letter_answers(&mut answers);
    assert_eq!(answers["empty_opts"], serde_json::Value::String("A".into()));
}

// ── allow_blocked inheritance ──────────────────────────────────────

#[test]
fn test_step_inherits_workflow_level_allow_blocked() {
    let wf = make_workflow_level_blocked_workflow();
    let mut run = make_test_run();
    run.workflow_id = wf.id.clone();
    run.definition_name = wf.name.clone();
    let mut handler = make_handler(run, wf);
    // Step 0 has allow_blocked=None, workflow has allow_blocked=true
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Blocked);
}

#[test]
fn test_step_allow_blocked_overrides_workflow_deny() {
    let mut handler = make_handler(make_test_run(), make_test_workflow());
    // Step 0 has allow_blocked=Some(true) but workflow has allow_blocked=false
    // Step-level override should win
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Blocked);
}

#[test]
fn test_step_none_inherits_workflow_false() {
    let mut run = make_test_run();
    run.current_step = 2; // step 2 has allow_blocked=None, workflow has allow_blocked=false
    let mut handler = make_handler(run, make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(!handler.process_tool_result(content).0);
    assert_eq!(typed_state(&handler).phase, Phase::Executing);
}
