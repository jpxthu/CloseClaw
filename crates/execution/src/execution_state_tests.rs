//! Tests for execution_state: state machine, PlanStateWriter, step_status_to_marker.

use crate::execution_state::{
    apply_transition, get_step_status, init_execution_steps, progress_summary,
    step_status_to_marker, validate_transition, DefaultPlanStateWriter, ExecutionState,
    PlanStateWriter,
};
use crate::{ExecutionStep, ExecutionStepStatus, TransitionError};

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
fn test_transition_skipped_to_in_progress() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["step1".into(), "step2".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(1));
    assert!(validate_transition(&state, 0, &ExecutionStepStatus::InProgress).is_ok());
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::InProgress)
    );
    assert_eq!(state.current_step, Some(0));
}

#[test]
fn test_skipped_to_in_progress_current_step_points_to_step() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into(), "c".into()]);
    state.current_step = Some(1);
    apply_transition(&mut state, 1, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(2));
    apply_transition(&mut state, 1, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(state.current_step, Some(1));
}

#[test]
fn test_skipped_to_in_progress_no_preset_current_step() {
    let mut state = ExecutionState::new();
    init_execution_steps(&mut state, vec!["a".into(), "b".into(), "c".into()]);
    state.current_step = Some(0);
    apply_transition(&mut state, 0, ExecutionStepStatus::Skipped).unwrap();
    assert_eq!(state.current_step, Some(1));
    assert!(validate_transition(&state, 0, &ExecutionStepStatus::InProgress).is_ok());
    apply_transition(&mut state, 0, ExecutionStepStatus::InProgress).unwrap();
    assert_eq!(state.current_step, Some(0));
    assert_eq!(
        get_step_status(&state, 0),
        Some(&ExecutionStepStatus::InProgress)
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
        TransitionError::OutOfBounds {
            index: 99,
            len: 1
        }
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
    assert_eq!(
        step_status_to_marker(&ExecutionStepStatus::Skipped),
        "[~]"
    );
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
    writer
        .write_progress_to_plan_file(&plan_path, &es)
        .unwrap();
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
    writer
        .write_progress_to_plan_file(&plan_path, &es)
        .unwrap();
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
    writer
        .write_progress_to_plan_file(&plan_path, &es)
        .unwrap();
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
    writer
        .write_progress_to_plan_file(&plan_path, &es)
        .unwrap();
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
    writer
        .write_progress_to_plan_file(&plan_path, &es)
        .unwrap();
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
