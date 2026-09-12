//! Serialization tests for WorkflowRun: paused_reason round-trip and
//! old checkpoint backward compatibility.

use crate::engine::WorkflowEngine;
use crate::run::{Phase, WorkflowRun};
use crate::test_fixtures::simple_workflow;

#[test]
fn test_paused_reason_round_trip_serde() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Blocked;
    run.paused_reason = "验收重试次数耗尽".to_string();

    let json = serde_json::to_string(&run).unwrap();
    let deserialized: WorkflowRun = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.paused_reason, "验收重试次数耗尽");
    assert_eq!(deserialized.phase, Phase::Blocked);
}

#[test]
fn test_old_checkpoint_without_paused_reason_deserializes_to_empty() {
    // Simulate old checkpoint JSON that has no paused_reason field.
    let old_json = r#"{
        "workflow_id": "test",
        "definition_name": "Test",
        "definition_version": "0.1",
        "current_step": 0,
        "phase": "blocked",
        "step_history": [],
        "step_data": null,
        "pending_goal_hint": "normal",
        "pending_verify": 3
    }"#;

    let deserialized: WorkflowRun = serde_json::from_str(old_json).unwrap();
    assert_eq!(deserialized.paused_reason, "");
    assert_eq!(deserialized.phase, Phase::Blocked);
}

#[test]
fn test_paused_reason_empty_round_trip() {
    let wf = simple_workflow();
    let run = WorkflowEngine::start(&wf);
    assert!(run.paused_reason.is_empty());

    let json = serde_json::to_string(&run).unwrap();
    let deserialized: WorkflowRun = serde_json::from_str(&json).unwrap();
    assert!(deserialized.paused_reason.is_empty());
}
