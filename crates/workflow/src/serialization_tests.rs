//! Serialization tests for WorkflowRun: paused_reason round-trip and
//! old checkpoint backward compatibility.

use crate::engine::WorkflowEngine;
use crate::run::{Phase, StepHistoryEntry, StepHistoryStatus, WorkflowRun};
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

// ---------------------------------------------------------------------------
// StepHistoryEntry backward compatibility
// ---------------------------------------------------------------------------

#[test]
fn test_step_history_entry_old_json_defaults_to_completed() {
    // Simulate old checkpoint JSON where StepHistoryEntry has no `status` field.
    let old_entry_json = r#"{
        "step_id": 0,
        "step_name": "Setup",
        "entered_at": "2026-09-13T06:00:00+00:00",
        "completed_at": "2026-09-13T06:05:00+00:00"
    }"#;

    let entry: StepHistoryEntry = serde_json::from_str(old_entry_json).unwrap();
    assert_eq!(entry.step_id, 0);
    assert_eq!(entry.step_name, "Setup");
    assert_eq!(entry.status, StepHistoryStatus::Completed);
}

#[test]
fn test_step_history_entry_round_trip_with_status() {
    let entry = StepHistoryEntry {
        step_id: 1,
        step_name: "Execute".into(),
        entered_at: "2026-09-13T06:00:00+00:00".into(),
        completed_at: "2026-09-13T06:10:00+00:00".into(),
        status: StepHistoryStatus::Skipped,
    };

    let json = serde_json::to_string(&entry).unwrap();
    let deserialized: StepHistoryEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.status, StepHistoryStatus::Skipped);
    assert_eq!(deserialized.step_id, 1);
}
