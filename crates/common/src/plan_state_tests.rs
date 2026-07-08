//! Tests for PlanState and PlanPhase types.

use super::*;
use crate::plan_state::PlanPath;

#[test]
fn test_plan_phase_default_is_research() {
    assert_eq!(PlanPhase::default(), PlanPhase::Research);
}

#[test]
fn test_plan_state_default() {
    let state = PlanState::default();
    assert_eq!(state.phase, PlanPhase::Research);
    assert!(state.pending_steps.is_empty());
    assert!(state.plan_file_path.is_empty());
}

#[test]
fn test_plan_state_new() {
    let state = PlanState::new();
    assert_eq!(state.phase, PlanPhase::Research);
    assert!(state.pending_steps.is_empty());
    assert!(state.plan_file_path.is_empty());
}

#[test]
fn test_plan_phase_all_variants() {
    let variants = [
        PlanPhase::Research,
        PlanPhase::Design,
        PlanPhase::Review,
        PlanPhase::FinalPlan,
        PlanPhase::Interview,
    ];
    assert_eq!(variants.len(), 5);
}

#[test]
fn test_plan_phase_serde_snake_case() {
    let cases = [
        (PlanPhase::Research, "\"research\""),
        (PlanPhase::Design, "\"design\""),
        (PlanPhase::Review, "\"review\""),
        (PlanPhase::FinalPlan, "\"final_plan\""),
        (PlanPhase::Interview, "\"interview\""),
    ];
    for (phase, expected_json) in cases {
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(
            json, expected_json,
            "phase {:?} should serialize to {}",
            phase, expected_json
        );
        let deserialized: PlanPhase = serde_json::from_str(expected_json).unwrap();
        assert_eq!(deserialized, phase);
    }
}

#[test]
fn test_plan_state_serde_roundtrip() {
    let state = PlanState {
        phase: PlanPhase::Design,
        pending_steps: vec!["step1".into(), "step2".into()],
        plan_file_path: "/tmp/plan.md".into(),
        ..PlanState::default()
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: PlanState = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.phase, PlanPhase::Design);
    assert_eq!(deserialized.pending_steps, vec!["step1", "step2"]);
    assert_eq!(deserialized.plan_file_path, "/tmp/plan.md");
}

#[test]
fn test_plan_state_serde_default_fields() {
    // Missing fields should use Default values
    let json = "{}";
    let state: PlanState = serde_json::from_str(json).unwrap();
    assert_eq!(state.phase, PlanPhase::Research);
    assert!(state.pending_steps.is_empty());
    assert!(state.plan_file_path.is_empty());
}

#[test]
fn test_plan_state_serialization_field_names_snake_case() {
    let state = PlanState::new();
    let json = serde_json::to_value(&state).unwrap();
    assert!(json.get("phase").is_some());
    assert!(json.get("pending_steps").is_some());
    assert!(json.get("plan_file_path").is_some());
}

#[test]
fn test_init_execution_steps() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into(), "step3".into()]);
    assert_eq!(state.execution_steps.len(), 3);
    assert!(state.current_step.is_none());
    for (i, step) in state.execution_steps.iter().enumerate() {
        assert_eq!(step.step_index, i);
        assert_eq!(step.status, ExecutionStepStatus::Pending);
        assert!(step.error_message.is_none());
    }
    assert_eq!(
        state.get_step_status(0),
        Some(&ExecutionStepStatus::Pending)
    );
    assert_eq!(
        state.get_step_status(2),
        Some(&ExecutionStepStatus::Pending)
    );
    assert_eq!(state.get_step_status(3), None);
}

#[test]
fn test_step_status_serde_roundtrip() {
    let statuses = [
        ExecutionStepStatus::Pending,
        ExecutionStepStatus::InProgress,
        ExecutionStepStatus::Completed,
        ExecutionStepStatus::Failed,
        ExecutionStepStatus::Skipped,
    ];
    for status in &statuses {
        let json = serde_json::to_string(status).unwrap();
        let deserialized: ExecutionStepStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, status);
    }
}

#[test]
fn test_transition_pending_to_in_progress() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    assert!(state
        .validate_transition(0, &ExecutionStepStatus::InProgress)
        .is_ok());
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    assert_eq!(
        state.get_step_status(0),
        Some(&ExecutionStepStatus::InProgress)
    );
}

#[test]
fn test_transition_in_progress_to_completed() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    assert_eq!(
        state.get_step_status(0),
        Some(&ExecutionStepStatus::Completed)
    );
    assert_eq!(state.current_step, Some(1));
}

#[test]
fn test_transition_in_progress_to_failed() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Failed)
        .unwrap();
    assert_eq!(state.get_step_status(0), Some(&ExecutionStepStatus::Failed));
    assert_eq!(state.current_step, Some(0));
}

#[test]
fn test_transition_failed_to_in_progress() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Failed)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    assert_eq!(
        state.get_step_status(0),
        Some(&ExecutionStepStatus::InProgress)
    );
}

#[test]
fn test_transition_completed_cannot_go_back() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    let err = state.validate_transition(0, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::InvalidTransition { .. }
    ));
}

#[test]
fn test_transition_skip_step_rejected() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into()]);
    let err = state.validate_transition(1, &ExecutionStepStatus::InProgress);
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
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    let err = state.validate_transition(5, &ExecutionStepStatus::InProgress);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        TransitionError::OutOfBounds { index: 5, len: 1 }
    ));
}

#[test]
fn test_transition_skipped_from_pending() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    assert!(state
        .validate_transition(0, &ExecutionStepStatus::Skipped)
        .is_ok());
    state
        .apply_transition(0, ExecutionStepStatus::Skipped)
        .unwrap();
    assert_eq!(
        state.get_step_status(0),
        Some(&ExecutionStepStatus::Skipped)
    );
}

#[test]
fn test_init_then_full_flow() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into(), "step3".into()]);
    // Step 0: pending → in_progress → completed
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    // Step 1: pending → in_progress → completed
    state.current_step = Some(1);
    state
        .apply_transition(1, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(1, ExecutionStepStatus::Completed)
        .unwrap();
    // Step 2: pending → in_progress → completed
    state.current_step = Some(2);
    state
        .apply_transition(2, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(2, ExecutionStepStatus::Completed)
        .unwrap();
    // All done
    for (i, step) in state.execution_steps.iter().enumerate() {
        assert_eq!(
            step.status,
            ExecutionStepStatus::Completed,
            "step {} should be Completed",
            i
        );
    }
    // current_step stays at last index (no next step)
    assert_eq!(state.current_step, Some(2));
}

// --- PlanPath tests ---

#[test]
fn test_plan_path_default_is_interview() {
    assert_eq!(PlanPath::default(), PlanPath::Interview);
}

#[test]
fn test_plan_path_all_variants() {
    let variants = [PlanPath::Standard, PlanPath::Interview];
    assert_eq!(variants.len(), 2);
}

#[test]
fn test_plan_path_serde_snake_case() {
    let cases = [
        (PlanPath::Standard, r#""standard""#),
        (PlanPath::Interview, r#""interview""#),
    ];
    for (path, expected_json) in cases {
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(
            json, expected_json,
            "path {:?} should serialize to {}",
            path, expected_json
        );
        let deserialized: PlanPath = serde_json::from_str(expected_json).unwrap();
        assert_eq!(deserialized, path);
    }
}

#[test]
fn test_plan_path_display() {
    assert_eq!(PlanPath::Standard.to_string(), "standard");
    assert_eq!(PlanPath::Interview.to_string(), "interview");
}

#[test]
fn test_plan_state_serde_with_explicit_path() {
    let state = PlanState {
        explicit_path: Some(PlanPath::Standard),
        ..PlanState::default()
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: PlanState = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.explicit_path, Some(PlanPath::Standard));
}

#[test]
fn test_plan_state_explicit_path_none_by_default() {
    let state = PlanState::new();
    assert_eq!(state.explicit_path, None);
}

#[test]
fn test_plan_state_serde_backward_compat_without_explicit_path() {
    // Old serialized state without explicit_path field should deserialize fine
    let json = r#"{"phase": "research", "plan_file_path": "/tmp/plan.md"}"#;
    let state: PlanState = serde_json::from_str(json).unwrap();
    assert_eq!(state.explicit_path, None);
    assert_eq!(state.phase, PlanPhase::Research);
    assert_eq!(state.plan_file_path, "/tmp/plan.md");
}

// --- progress_summary tests ---

#[test]
fn test_progress_summary_empty_steps() {
    let state = PlanState::new();
    assert_eq!(state.progress_summary(), "");
}

#[test]
fn test_progress_summary_single_pending() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    let summary = state.progress_summary();
    assert!(summary.contains("## Execution Progress"));
    assert!(summary.contains("Step 1/1: pending"));
}

#[test]
fn test_progress_summary_single_completed() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["do stuff".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/1: completed (do stuff)"));
}

#[test]
fn test_progress_summary_completed_no_summary() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/1: completed"));
}

#[test]
fn test_progress_summary_multi_mixed() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into(), "step3".into()]);
    // Step 0 completed (auto-advances current_step to 1)
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Completed)
        .unwrap();
    // Step 1 in_progress (current_step already == 1)
    state
        .apply_transition(1, ExecutionStepStatus::InProgress)
        .unwrap();
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/3: completed (step1)"));
    assert!(summary.contains("→ Step 2/3: in_progress"));
    assert!(summary.contains("Step 3/3: pending"));
    // Arrow only on current step
    let lines: Vec<&str> = summary.lines().collect();
    assert!(lines[1].starts_with("Step 1"));
    assert!(lines[2].starts_with("→ Step 2"));
    assert!(lines[3].starts_with("Step 3"));
}

#[test]
fn test_progress_summary_failed_with_error() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Failed)
        .unwrap();
    state.execution_steps[0].error_message = Some("timeout".into());
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/1: failed (timeout)"));
}

#[test]
fn test_progress_summary_failed_no_error() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state.current_step = Some(0);
    state
        .apply_transition(0, ExecutionStepStatus::InProgress)
        .unwrap();
    state
        .apply_transition(0, ExecutionStepStatus::Failed)
        .unwrap();
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/1: failed"));
}

#[test]
fn test_progress_summary_skipped() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into()]);
    state
        .apply_transition(0, ExecutionStepStatus::Skipped)
        .unwrap();
    let summary = state.progress_summary();
    assert!(summary.contains("Step 1/1: skipped"));
}

#[test]
fn test_progress_summary_no_current_step() {
    let mut state = PlanState::new();
    state.init_execution_steps(vec!["step1".into(), "step2".into()]);
    // current_step is None — no arrow
    let summary = state.progress_summary();
    let lines: Vec<&str> = summary.lines().collect();
    assert!(lines[1].starts_with("Step 1"));
    assert!(lines[2].starts_with("Step 2"));
}

// ---------------------------------------------------------------------------
// DefaultPlanStateWriter tests
// ---------------------------------------------------------------------------

fn make_plan_file(dir: &std::path::Path, step_names: &[&str]) -> String {
    let path = dir.join("plan.md");
    let mut content = String::from("# Plan\n\n## 进度\n\n");
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

    let mut ps = PlanState::new();
    ps.plan_file_path = plan_path.clone();
    ps.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::InProgress,
        summary: "Step 1".into(),
        error_message: None,
    });

    writer.write_progress_to_plan_file(&plan_path, &ps).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        content.contains("\u{1f504}"),
        "expected \u{1f504} marker: {content}"
    );
}

#[test]
fn test_writer_updates_completed_marker() {
    let dir = std::env::temp_dir().join("cc_test_writer_completed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan_path = make_plan_file(&dir, &["1.1"]);
    let writer = DefaultPlanStateWriter::new();

    let mut ps = PlanState::new();
    ps.plan_file_path = plan_path.clone();
    ps.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });

    writer.write_progress_to_plan_file(&plan_path, &ps).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        content.contains("\u{2705}"),
        "expected \u{2705} marker: {content}"
    );
}

#[test]
fn test_writer_updates_failed_marker() {
    let dir = std::env::temp_dir().join("cc_test_writer_failed");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan_path = make_plan_file(&dir, &["1.1"]);
    let writer = DefaultPlanStateWriter::new();

    let mut ps = PlanState::new();
    ps.plan_file_path = plan_path.clone();
    ps.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "Step 1".into(),
        error_message: None,
    });

    writer.write_progress_to_plan_file(&plan_path, &ps).unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        content.contains("\u{274c}"),
        "expected \u{274c} marker: {content}"
    );
}

#[test]
fn test_writer_file_not_found() {
    let writer = DefaultPlanStateWriter::new();
    let ps = PlanState::new();
    let result = writer.write_progress_to_plan_file("/nonexistent/path.md", &ps);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_writer_preserves_non_step_content() {
    let dir = std::env::temp_dir().join("cc_test_writer_preserve");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.md");
    let content = concat!(
        "# Plan\n",
        "\n",
        "Keep this.\n",
        "\n",
        "## \u{8fdb}\u{5ea6}\n",
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
    let mut ps = PlanState::new();
    ps.plan_file_path = plan_path.clone();
    ps.execution_steps.push(ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });

    writer.write_progress_to_plan_file(&plan_path, &ps).unwrap();
    let result = std::fs::read_to_string(&plan_path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(result.contains("# Plan"));
    assert!(result.contains("Keep this."));
    assert!(result.contains("## Notes"));
    assert!(result.contains("More notes."));
    assert!(result.contains("\u{2705}"));
}
