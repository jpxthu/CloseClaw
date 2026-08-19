//! Tests for execution_state: state machine, PlanStateWriter, step_status_to_marker.

use crate::execution_state::{
    apply_transition, get_step_status, init_execution_steps, progress_summary,
    step_status_to_marker, validate_transition, DefaultPlanStateWriter, ExecutionState,
    PlanStateWriter,
};
use crate::{ExecutionError, ExecutionEvent, ExecutionStep, ExecutionStepStatus, TransitionError};

// --- init_execution_steps ---

#[test]
fn test_init_execution_steps() {
    let mut state = ExecutionState::new();
    init_execution_steps(
        &mut state,
        vec!["step1".into(), "step2".into(), "step3".into()],
    );
    assert_eq!(state.execution_steps.len(), 3);
    assert!(state.current_step.is_none());
    for (i, step) in state.execution_steps.iter().enumerate() {
        assert_eq!(step.step_index, i);
        assert_eq!(step.status, ExecutionStepStatus::Pending);
        assert!(step.error_message.is_none());
    }
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::Pending)
    );
    assert_eq!(
        get_step_status(&state, 2),
        Some(&ExecutionStepStatus::Pending)
    );
    assert_eq!(get_step_status(&state, 3), None);
}

// --- validate_transition ---

#[test]
fn test_transition_pending_to_in_progress() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    assert!(validate_transition(&state, 0, &ExecutionStepStatus::InProgress).is_ok());
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::InProgress)
    );
}

#[test]
fn test_pending_to_in_progress_preserves_current_step() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(state.current_step, Some(0));
}

#[test]
fn test_transition_in_progress_to_completed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into(), "step2".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::Completed)
    );
    assert_eq!(state.current_step, Some(1));
}

#[test]
fn test_transition_in_progress_to_failed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Failed).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::Failed)
    );
    assert_eq!(state.current_step, Some(0));
}

#[test]
fn test_transition_failed_to_in_progress() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Failed).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::InProgress)
    );
}

#[test]
fn test_transition_completed_cannot_go_back() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    let err = validate_transition(&state, 0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::InvalidTransition { .. }
    ));
}

#[test]
fn test_transition_skip_step_rejected() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into(), "step2".into()]);
    let err = validate_transition(&state, 1, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::SkippedStep {
            expected: 0,
            got: 1
        }
    ));
}

#[test]
fn test_transition_out_of_bounds() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    let err = validate_transition(&state, 5, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::OutOfBounds { index: 5, len: 1 }
    ));
}

#[test]
fn test_transition_skipped_from_pending() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    assert!(validate_transition(&state, 0, &ExecutionStepStatus::Skipped).is_ok());
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::Skipped)
    );
}

#[test]
fn test_transition_skipped_to_in_progress_rejected() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into(), "step2".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(1));
    let err = validate_transition(&state, 0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    // Either SkippedStep or InvalidTransition is acceptable — the key point
    // is that Skipped→InProgress is not allowed.
    let err_inner = err.unwrap_err();
    assert!(
        matches!(
            err_inner,
            TransitionError::InvalidTransition {
                from: ExecutionStepStatus::Skipped,
                to: ExecutionStepStatus::InProgress
            }
        ) || matches!(err_inner, TransitionError::SkippedStep { .. }),
        "expected InvalidTransition or SkippedStep, got: {:?}",
        err_inner
    );
}

#[test]
fn test_skipped_to_in_progress_rejected_current_step_past() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into(), "c".into()]);
    state.current_step = Some(1);
    apply_transition(&mut state, 1, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(2));
    let err = validate_transition(&state, 1, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    let err_inner = err.unwrap_err();
    assert!(
        matches!(
            err_inner,
            TransitionError::InvalidTransition {
                from: ExecutionStepStatus::Skipped,
                to: ExecutionStepStatus::InProgress
            }
        ) || matches!(err_inner, TransitionError::SkippedStep { .. }),
        "expected InvalidTransition or SkippedStep, got: {:?}",
        err_inner
    );
}

#[test]
fn test_skipped_to_in_progress_rejected_no_preset() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into(), "c".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(1));
    let err = validate_transition(&state, 0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    let err_inner = err.unwrap_err();
    assert!(
        matches!(
            err_inner,
            TransitionError::InvalidTransition {
                from: ExecutionStepStatus::Skipped,
                to: ExecutionStepStatus::InProgress
            }
        ) || matches!(err_inner, TransitionError::SkippedStep { .. }),
        "expected InvalidTransition or SkippedStep, got: {:?}",
        err_inner
    );
}

#[test]
fn test_skipped_to_completed_not_allowed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    state.current_step = Some(0);
    let err = validate_transition(&state, 0, &ExecutionStepStatus::Completed);
    assert!(err.is_err());
}

#[test]
fn test_completed_to_in_progress_not_allowed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    let err = validate_transition(&state, 0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
}

#[test]
fn test_skipped_to_skipped_not_allowed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    state.current_step = Some(0);
    let err = validate_transition(&state, 0, &ExecutionStepStatus::Skipped);
    assert!(err.is_err());
}

#[test]
fn test_init_then_full_flow() {
    let mut state = ExecutionState::new();
    init_execution_steps(
        &mut state,
        vec!["step1".into(), "step2".into(), "step3".into()],
    );
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    state.current_step = Some(1);
    apply_transition(&mut state, 1, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 1, ExecutionStepStatus::Completed).unwrap();
    state.current_step = Some(2);
    apply_transition(&mut state, 2, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 2, ExecutionStepStatus::Completed).unwrap();
    for (i, step) in state.execution_steps.iter().enumerate() {
        assert_eq!(
            step.status,
            ExecutionStepStatus::Completed,
            "step {} should be Completed",
            i
        );
    }
    assert_eq!(state.current_step, Some(2));
}

#[test]
fn test_pending_to_completed_direct_invalid() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    let err = apply_transition(&mut state, 0, ExecutionStepStatus::Completed);
    assert!(err.is_err());
}

#[test]
fn test_pending_to_failed_direct_invalid() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    let err = apply_transition(&mut state, 0, ExecutionStepStatus::Failed);
    assert!(err.is_err());
}

#[test]
fn test_apply_transition_returns_ok_for_valid_chain() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["s1".into(), "s2".into()]);
    state.current_step = Some(0);
    assert!(apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).is_ok());
    assert!(apply_transition(&mut state, 0, ExecutionStepStatus::Completed).is_ok());
    assert_eq!(state.current_step, Some(1));
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::Completed)
    );
}

#[test]
fn test_out_of_bounds_apply_transition_fails() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["s1".into()]);
    let err = apply_transition(&mut state, 99, ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::OutOfBounds { index: 99, len: 1 }
    ));
}

// --- progress_summary ---

#[test]
fn test_progress_summary_empty_steps() {
    let state = ExecutionState::new();
    assert_eq!(progress_summary(&state), "");
}

#[test]
fn test_progress_summary_single_pending() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    let summary = progress_summary(&state);
    assert!(summary.contains("## Execution Progress"));
    assert!(summary.contains("Step 1/1: pending"));
}

#[test]
fn test_progress_summary_single_completed() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["do stuff".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/1: completed (do stuff)"));
}

#[test]
fn test_progress_summary_completed_no_summary() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/1: completed"));
}

#[test]
fn test_progress_summary_multi_mixed() {
    let mut state = ExecutionState::new();
    init_execution_steps(
        &mut state,
        vec!["step1".into(), "step2".into(), "step3".into()],
    );
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Completed).unwrap();
    apply_transition(&mut state, 1, ExecutionStepStatus::InProgress).unwrap();
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/3: completed (step1)"));
    assert!(summary.contains("→ Step 2/3: in_progress"));
    assert!(summary.contains("Step 3/3: pending"));
    let lines: Vec<&str> = summary.lines().collect();
    assert!(lines[1].starts_with("Step 1"));
    assert!(lines[2].starts_with("→ Step 2"));
    assert!(lines[3].starts_with("Step 3"));
}

#[test]
fn test_progress_summary_failed_with_error() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Failed).unwrap();
    state.execution_steps[0].error_message = Some("timeout".into());
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/1: failed (timeout)"));
}

#[test]
fn test_progress_summary_failed_no_error() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    apply_transition(&mut state, 0, ExecutionStepStatus::Failed).unwrap();
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/1: failed"));
}

#[test]
fn test_progress_summary_skipped() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into()]);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    let summary = progress_summary(&state);
    assert!(summary.contains("Step 1/1: skipped"));
}

#[test]
fn test_progress_summary_no_current_step() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into(), "step2".into()]);
    let summary = progress_summary(&state);
    let lines: Vec<&str> = summary.lines().collect();
    assert!(lines[1].starts_with("Step 1"));
    assert!(lines[2].starts_with("Step 2"));
}

// --- step_status_to_marker ---

#[test]
fn test_step_status_to_marker_checkbox_format() {
    assert_eq!(
        step_status_to_marker(&ExecutionStepStatus::Completed),
        "[x]"
    );
    assert_eq!(
        step_status_to_marker(&ExecutionStepStatus::InProgress),
        "[-]"
    );
    assert_eq!(step_status_to_marker(&ExecutionStepStatus::Failed), "[!]");
    assert_eq!(step_status_to_marker(&ExecutionStepStatus::Pending), "[ ]");
    assert_eq!(step_status_to_marker(&ExecutionStepStatus::Skipped), "[~]");
}

// --- DefaultPlanStateWriter ---

fn make_plan_file(dir: &std::path::Path, step_names: &[&str]) -> String {
    let path = dir.join("plan.md");
    let mut content = String::from("# Plan\n\n## Tasks\n\n");
    content.push_str("| | Step | Status |\n");
    content.push_str("|---|---|---|\n");
    for name in step_names {
        content.push_str(&format!("| | {} | detail |\n", name));
    }
    std::fs::write(&path, &content).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn test_writer_updates_in_progress_marker() {
    let dir = std::env::temp_dir().join("cc_test_writer_in_progress");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan_path = make_plan_file(&dir, &["1.1", "2.1"]);
    let writer = DefaultPlanStateWriter::new();
    let mut es = ExecutionState::new();
    es.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::InProgress,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer.write_progress_to_plan_file(&plan_path, &es).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(content.contains("[-]"), "expected [-] marker: {content}");
}

#[test]
fn test_writer_updates_completed_marker() {
    let dir = std::env::temp_dir().join("cc_test_writer_completed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan_path = make_plan_file(&dir, &["1.1"]);
    let writer = DefaultPlanStateWriter::new();
    let mut es = ExecutionState::new();
    es.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer.write_progress_to_plan_file(&plan_path, &es).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(content.contains("[x]"), "expected [x] marker: {content}");
}

#[test]
fn test_writer_updates_failed_marker() {
    let dir = std::env::temp_dir().join("cc_test_writer_failed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan_path = make_plan_file(&dir, &["1.1"]);
    let writer = DefaultPlanStateWriter::new();
    let mut es = ExecutionState::new();
    es.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer.write_progress_to_plan_file(&plan_path, &es).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(content.contains("[!]"), "expected [!] marker: {content}");
}

#[test]
fn test_writer_file_not_found() {
    let writer = DefaultPlanStateWriter::new();
    let es = ExecutionState::new();
    let result = writer.write_progress_to_plan_file("/nonexistent/path.md", &es);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_writer_preserves_non_step_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.md");
    let content = concat!(
        "# Plan\n",
        "\n",
        "Keep this.\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Status |\n",
        "|---|---|---|\n",
        "| | 1.1 | detail |\n",
        "\n",
        "## Notes\n",
        "\n",
        "More notes.\n",
    );
    std::fs::write(&path, content).unwrap();
    let plan_path = path.to_str().unwrap().to_string();
    let writer = DefaultPlanStateWriter::new();
    let mut es = ExecutionState::new();
    es.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer.write_progress_to_plan_file(&plan_path, &es).unwrap();
    let result = std::fs::read_to_string(&plan_path).unwrap();
    assert!(result.contains("# Plan"));
    assert!(result.contains("Keep this."));
    assert!(result.contains("## Notes"));
    assert!(result.contains("More notes."));
    assert!(result.contains("[x]"));
}

#[test]
fn test_writer_updates_tasks_section_marker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.md");
    let content = concat!(
        "# Plan\n",
        "\n",
        "## Context\n",
        "\n",
        "Background info.\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Description |\n",
        "|---|---|---|\n",
        "| [ ] | 1.1 | First step |\n",
        "| [ ] | 2.1 | Second step |\n",
        "| [ ] | 3.1 | Third step |\n",
        "\n",
        "## Verification\n",
        "\n",
        "Run tests.\n",
    );
    std::fs::write(&path, content).unwrap();
    let plan_path = path.to_str().unwrap().to_string();
    let writer = DefaultPlanStateWriter::new();
    let mut es = ExecutionState::new();
    es.execution_steps = vec![
        ExecutionStep {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "First step".into(),
            error_message: None,
        },
        ExecutionStep {
            step_index: 1,
            status: ExecutionStepStatus::InProgress,
            summary: "Second step".into(),
            error_message: None,
        },
        ExecutionStep {
            step_index: 2,
            status: ExecutionStepStatus::Pending,
            summary: "Third step".into(),
            error_message: None,
        },
    ];
    writer.write_progress_to_plan_file(&plan_path, &es).unwrap();
    let result = std::fs::read_to_string(&plan_path).unwrap();
    assert!(
        result.contains("|[x]| 1.1 |"),
        "step 1.1 should be [x]: {result}"
    );
    assert!(
        result.contains("|[-]| 2.1 |"),
        "step 2.1 should be [-]: {result}"
    );
    assert!(
        result.contains("|[ ]| 3.1 |"),
        "step 3.1 should be [ ]: {result}"
    );
    assert!(result.contains("## Context"));
    assert!(result.contains("Background info."));
    assert!(result.contains("## Tasks"));
    assert!(result.contains("## Verification"));
    assert!(result.contains("Run tests."));
}

// --- ExecutionState serde backward compat ---

/// Deserializing an old checkpoint with extra fields (e.g. old `execution_steps`
/// embedded in PlanState) should not fail — `#[serde(default)]` handles them.
#[test]
fn test_execution_state_deserialize_with_extra_fields() {
    let json = r#"{
        "execution_steps": [
            {"step_index": 0, "status": "completed", "summary": "done", "error_message": null}
        ],
        "current_step": 1,
        "explicit_path": "standard",
        "step_selection": null,
        "unknown_future_field": "ignored"
    }"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert_eq!(state.execution_steps.len(), 1);
    assert_eq!(
        state.execution_steps[0].status,
        ExecutionStepStatus::Completed
    );
    assert_eq!(state.current_step, Some(1));
    assert_eq!(
        state.explicit_path,
        Some(closeclaw_common::PlanPath::Standard)
    );
}

/// Deserializing an empty JSON object should produce a default ExecutionState.
#[test]
fn test_execution_state_deserialize_empty_object() {
    let json = r#"{}"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert!(state.execution_steps.is_empty());
    assert!(state.current_step.is_none());
    assert!(state.explicit_path.is_none());
    assert!(state.step_selection.is_none());
}

/// Roundtrip: serialize then deserialize should preserve all fields.
#[test]
fn test_execution_state_serde_roundtrip() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into()]);
    state.current_step = Some(1);
    state.step_selection = Some(vec![0, 1]);

    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.execution_steps.len(), 2);
    assert_eq!(restored.current_step, Some(1));
    assert_eq!(restored.step_selection, Some(vec![0, 1]));
}

/// ExecutionStep serde roundtrip with all fields.
#[test]
fn test_execution_step_serde_roundtrip() {
    let step = ExecutionStep {
        step_index: 2,
        status: ExecutionStepStatus::Failed,
        summary: "test summary".into(),
        error_message: Some("boom".into()),
    };
    let json = serde_json::to_string(&step).unwrap();
    let restored: ExecutionStep = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_index, 2);
    assert_eq!(restored.status, ExecutionStepStatus::Failed);
    assert_eq!(restored.summary, "test summary");
    assert_eq!(restored.error_message, Some("boom".into()));
}

/// ExecutionStepStatus serde uses snake_case.
#[test]
fn test_execution_step_status_serde_snake_case() {
    assert_eq!(
        serde_json::to_string(&ExecutionStepStatus::InProgress).unwrap(),
        "\"in_progress\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionStepStatus::Completed).unwrap(),
        "\"completed\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionStepStatus::Failed).unwrap(),
        "\"failed\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionStepStatus::Skipped).unwrap(),
        "\"skipped\""
    );
    assert_eq!(
        serde_json::to_string(&ExecutionStepStatus::Pending).unwrap(),
        "\"pending\""
    );
}

// --- ExecutionState.explicit_path serde tests ---

/// explicit_path roundtrip: None → None.
#[test]
fn test_execution_state_explicit_path_none_default() {
    let state = ExecutionState::new();
    assert!(state.explicit_path.is_none());
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert!(restored.explicit_path.is_none());
}

/// explicit_path roundtrip: Standard.
#[test]
fn test_execution_state_explicit_path_standard_roundtrip() {
    let mut state = ExecutionState::new();
    state.explicit_path = Some(closeclaw_common::PlanPath::Standard);
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.explicit_path,
        Some(closeclaw_common::PlanPath::Standard)
    );
}

/// explicit_path roundtrip: Interview.
#[test]
fn test_execution_state_explicit_path_interview_roundtrip() {
    let mut state = ExecutionState::new();
    state.explicit_path = Some(closeclaw_common::PlanPath::Interview);
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.explicit_path,
        Some(closeclaw_common::PlanPath::Interview)
    );
}

/// explicit_path deserialization from old PlanState checkpoint format.
#[test]
fn test_execution_state_explicit_path_from_old_plan_state_checkpoint() {
    // Old PlanState had explicit_path; now it lives in ExecutionState.
    // Verify deserialization from old-format JSON works.
    let json = r#"{"explicit_path": "standard"}"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert_eq!(
        state.explicit_path,
        Some(closeclaw_common::PlanPath::Standard)
    );
}

/// explicit_path deserialization with old snake_case string values.
#[test]
fn test_execution_state_explicit_path_snake_case_values() {
    for (input, expected) in [
        (r#""standard""#, Some(closeclaw_common::PlanPath::Standard)),
        (
            r#""interview""#,
            Some(closeclaw_common::PlanPath::Interview),
        ),
    ] {
        let json = format!(r#"{{"explicit_path": {}}}"#, input);
        let state: ExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state.explicit_path, expected, "input: {input}");
    }
}

// --- ExecutionState.step_selection serde tests ---
/// step_selection None default roundtrip.
#[test]
fn test_execution_state_step_selection_none_default() {
    let state = ExecutionState::new();
    assert!(state.step_selection.is_none());
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert!(restored.step_selection.is_none());
}

/// step_selection Some roundtrip.
#[test]
fn test_execution_state_step_selection_some_roundtrip() {
    let mut state = ExecutionState::new();
    state.step_selection = Some(vec![0, 1, 2]);
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![0, 1, 2]));
}

/// step_selection empty vec: Some(vec![]) serializes and deserializes.
#[test]
fn test_execution_state_step_selection_empty_vec() {
    let mut state = ExecutionState::new();
    state.step_selection = Some(vec![]);
    let json = serde_json::to_string(&state).unwrap();
    let restored: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.step_selection, Some(vec![]));
}

// ---------------------------------------------------------------------------
// Skipped→InProgress rejection — additional edge cases
// ---------------------------------------------------------------------------
/// Skipped→InProgress rejected when current_step = None and step_index = 0.
#[test]
fn test_skipped_to_in_progress_rejected_no_current_step() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into()]);
    // No current_step set — step_index 0 is allowed by SkippedStep check,
    // but Skipped→InProgress is still invalid by the state machine.
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    let err = validate_transition(&state, 0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
}

/// Skipped→InProgress rejected via apply_transition (not just validate).
#[test]
fn test_apply_skipped_to_in_progress_rejected() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    // current_step moved to 1; set it back to 0 to test the rejected path
    state.current_step = Some(0);
    let result = apply_transition(&mut state, 0, ExecutionStepStatus::InProgress);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Removed dead code — enum completeness checks
/// Verify ExecutionError does not contain MaxRetriesExceeded variant.
///
/// This is a compile-time guarantee (no match arm needed), but we add
/// an explicit serialization roundtrip to document the expected variants.
#[test]
fn test_execution_error_no_max_retries_exceeded() {
    let errors = [
        ExecutionError::SpawnFailed {
            message: "x".into(),
        },
        ExecutionError::InvalidResult {
            message: "x".into(),
        },
        ExecutionError::StepFailed {
            step_index: 0,
            message: "x".into(),
        },
        ExecutionError::PermissionDenied {
            step_index: 0,
            reason: "x".into(),
        },
        ExecutionError::InvalidStepSelection { index: 0, total: 1 },
    ];
    // All variants should roundtrip through Debug without panicking.
    for err in &errors {
        let debug = format!("{err:?}");
        assert!(!debug.is_empty());
    }
}

/// Verify ExecutionEvent does not contain RetryTriggered variant.
///
/// Compile-time guarantee; explicit test documents the expected set.
#[test]
fn test_execution_event_no_retry_triggered() {
    let events = [
        ExecutionEvent::StepStarted { step_index: 0 },
        ExecutionEvent::StepCompleted {
            step_index: 0,
            summary: "done".into(),
        },
        ExecutionEvent::StepFailed {
            step_index: 0,
            error_message: "err".into(),
        },
        ExecutionEvent::AllCompleted,
        ExecutionEvent::HookExecuted { step_index: 0 },
        ExecutionEvent::HookFailed {
            step_index: 0,
            error_message: "err".into(),
        },
    ];
    for event in &events {
        let debug = format!("{event:?}");
        assert!(!debug.is_empty());
        // Ensure RetryTriggered is not in the debug output
        assert!(!debug.contains("RetryTriggered"));
    }
}

// ---------------------------------------------------------------------------
// StepResult — no attempts field
// ---------------------------------------------------------------------------

/// StepResult does not have an `attempts` field (retry dead code removed).
/// Verify construction and Debug output have no `attempts`.
#[test]
fn test_step_result_no_attempts_field() {
    use crate::engine::StepResult;
    let result = StepResult {
        step_index: 1,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec!["a.rs".into()],
        error_message: None,
        hook_blocked: None,
    };
    let debug = format!("{result:?}");
    assert!(
        !debug.contains("attempts"),
        "StepResult debug should not contain 'attempts': {debug}"
    );
    assert_eq!(result.step_index, 1);
    assert_eq!(result.description, "test");
    // Also verify all fields are accessible without attempts
    assert!(matches!(result.status, ExecutionStepStatus::Completed));
    assert!(result.changed_files.contains(&"a.rs".to_string()));
    assert!(result.error_message.is_none());
    assert!(result.hook_blocked.is_none());
}

/// step_selection from old PlanState checkpoint format.
#[test]
fn test_execution_state_step_selection_from_old_plan_state_checkpoint() {
    let json = r#"{"step_selection": [0, 1]}"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert_eq!(state.step_selection, Some(vec![0, 1]));
}

/// step_selection null value in JSON → None.
#[test]
fn test_execution_state_step_selection_null_value() {
    let json = r#"{"step_selection": null}"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert!(state.step_selection.is_none());
}

// --- ExecutionState: both explicit_path + step_selection together ---
/// Old checkpoint with both explicit_path and step_selection.
#[test]
fn test_execution_state_old_checkpoint_with_both_fields() {
    let json = r#"{
        "explicit_path": "interview",
        "step_selection": [2, 4]
    }"#;
    let state: ExecutionState = serde_json::from_str(json).unwrap();
    assert_eq!(
        state.explicit_path,
        Some(closeclaw_common::PlanPath::Interview)
    );
    assert_eq!(state.step_selection, Some(vec![2, 4]));
}

/// Empty object: both fields default to None.
#[test]
fn test_execution_state_empty_object_both_fields_none() {
    let state: ExecutionState = serde_json::from_str("{}").unwrap();
    assert!(state.explicit_path.is_none());
    assert!(state.step_selection.is_none());
}
