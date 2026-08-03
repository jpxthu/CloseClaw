//! Tests for workflow handler tool result processing.

use closeclaw_common::ContentBlock;
use closeclaw_workflow::definition::{Step, Workflow};
use closeclaw_workflow::run::{Phase, WorkflowRun};

use crate::workflow_handler::WorkflowHandler;

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
        ],
    }
}

fn make_test_run() -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test Workflow".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_verify: 0,
    }
}

#[test]
fn test_process_tool_result_start() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_start", "name": "Test Workflow"}"#;
    assert!(handler.process_tool_result(content));
    assert_eq!(handler.run().phase, Phase::Executing);
}

#[test]
fn test_process_tool_result_verify_no_transitions() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_verify"}"#;
    // No jump questions in step 0 and no transitions → NoMatchingTransition error → returns false
    assert!(!handler.process_tool_result(content));
}

#[test]
fn test_process_tool_result_blocked_allowed() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(handler.process_tool_result(content));
    assert_eq!(handler.run().phase, Phase::Blocked);
    let notif = handler.take_notification();
    assert!(notif.is_some());
    let notif = notif.unwrap();
    assert_eq!(notif.workflow_name, "Test Workflow");
    assert_eq!(notif.current_step, 0);
    assert!(notif.reason.contains("need help"));
}

#[test]
fn test_process_tool_result_blocked_not_allowed() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.run_mut().current_step = 1; // step 1 has allow_blocked = false
    let content = r#"{"action": "workflow_blocked", "reason": "need help"}"#;
    assert!(!handler.process_tool_result(content));
    assert_eq!(handler.run().phase, Phase::Executing);
}

#[test]
fn test_process_tool_result_unknown_action() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let content = r#"{"action": "unknown_action"}"#;
    assert!(!handler.process_tool_result(content));
}

#[test]
fn test_process_tool_result_invalid_json() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    assert!(!handler.process_tool_result("not json"));
}

#[test]
fn test_process_content_blocks() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let blocks = vec![
        ContentBlock::Text("some text".to_string()),
        ContentBlock::ToolResult {
            tool_call_id: "call-1".to_string(),
            content: r#"{"action": "workflow_blocked", "reason": "test"}"#.to_string(),
        },
    ];
    assert!(handler.process_content_blocks(&blocks));
    assert_eq!(handler.run().phase, Phase::Blocked);
}

#[test]
fn test_process_content_blocks_no_workflow() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let blocks = vec![ContentBlock::ToolResult {
        tool_call_id: "call-1".to_string(),
        content: r#"{"action": "some_other_tool"}"#.to_string(),
    }];
    assert!(!handler.process_content_blocks(&blocks));
    assert_eq!(handler.run().phase, Phase::Executing);
}

#[test]
fn test_on_owner_resolve() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.run_mut().phase = Phase::Blocked;
    handler.run_mut().pending_verify = 3;
    handler.on_owner_resolve();
    assert_eq!(handler.run().phase, Phase::Verifying);
    assert_eq!(handler.run().pending_verify, 0);
}

#[test]
fn test_on_owner_terminate() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.run_mut().phase = Phase::Blocked;
    handler.on_owner_terminate();
    assert_eq!(handler.run().phase, Phase::Complete);
}

#[test]
fn test_is_blocked() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    assert!(!handler.is_blocked());
    handler.run_mut().phase = Phase::Blocked;
    assert!(handler.is_blocked());
}

#[test]
fn test_is_complete() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    assert!(!handler.is_complete());
    handler.run_mut().phase = Phase::Complete;
    assert!(handler.is_complete());
}

#[test]
fn test_on_goal_injected() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.on_goal_injected();
    // step_data is cleared by on_goal_injected
    assert!(handler.run().step_data.is_null());
}

#[test]
fn test_notification_taken_only_once() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    let content = r#"{"action": "workflow_blocked", "reason": "test"}"#;
    handler.process_tool_result(content);
    assert!(handler.take_notification().is_some());
    assert!(handler.take_notification().is_none());
}

#[test]
fn test_on_verify_limit_exceeded() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.on_verify_injected(3);
    assert_eq!(handler.run().pending_verify, 1);
    assert_eq!(handler.run().phase, Phase::Executing);

    handler.on_verify_injected(3);
    assert_eq!(handler.run().pending_verify, 2);

    handler.on_verify_injected(3);
    assert_eq!(handler.run().pending_verify, 3);

    // 4th call exceeds limit of 3
    handler.on_verify_injected(3);
    assert_eq!(handler.run().phase, Phase::Blocked);
    assert!(handler.take_notification().is_some());
}

#[test]
fn test_on_verify_injected_within_limit() {
    let mut handler = WorkflowHandler::new(make_test_run(), make_test_workflow());
    handler.on_verify_injected(5);
    assert_eq!(handler.run().pending_verify, 1);
    assert_eq!(handler.run().phase, Phase::Executing);
    assert!(handler.take_notification().is_none());
}
