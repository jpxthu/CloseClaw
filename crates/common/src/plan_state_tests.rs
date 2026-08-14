//! Tests for PlanState, PlanPhase, and PlanPath basics.

use super::*;

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
fn test_plan_path_default_is_standard() {
    assert_eq!(PlanPath::default(), PlanPath::Standard);
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

// --- ExecutionStepStatus serde tests (type stays in common for serde compat) ---

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

// --- PlanState backward compat serde tests ---

#[test]
fn test_plan_state_serde_backward_compat_with_extra_fields() {
    // Old serialized data may contain execution_steps, current_step, etc.
    // serde should ignore unknown fields gracefully.
    let json = r#"{"phase": "research", "plan_file_path": "/tmp/plan.md", "execution_steps": [], "current_step": 0}"#;
    let state: PlanState = serde_json::from_str(json).unwrap();
    assert_eq!(state.phase, PlanPhase::Research);
    assert_eq!(state.plan_file_path, "/tmp/plan.md");
    assert!(state.pending_steps.is_empty());
}

#[test]
fn test_plan_state_serde_backward_compat_old_checkpoint() {
    // Old checkpoints may contain explicit_path and step_selection fields.
    // These should be silently ignored during deserialization.
    let json = r#"{"phase": "research", "plan_file_path": "/tmp/plan.md", "explicit_path": "standard", "step_selection": [0, 1]}"#;
    let state: PlanState = serde_json::from_str(json).unwrap();
    assert_eq!(state.phase, PlanPhase::Research);
    assert_eq!(state.plan_file_path, "/tmp/plan.md");
    assert!(state.pending_steps.is_empty());
}

// --- TransitionError tests (type stays in common) ---

#[test]
fn test_transition_error_display_out_of_bounds() {
    let err = TransitionError::OutOfBounds { index: 5, len: 3 };
    assert!(err.to_string().contains("5"));
    assert!(err.to_string().contains("3"));
}

#[test]
fn test_transition_error_display_invalid_transition() {
    let err = TransitionError::InvalidTransition {
        from: ExecutionStepStatus::Pending,
        to: ExecutionStepStatus::Completed,
    };
    assert!(err.to_string().contains("Pending"));
    assert!(err.to_string().contains("Completed"));
}

#[test]
fn test_transition_error_display_skipped_step() {
    let err = TransitionError::SkippedStep {
        expected: 0,
        got: 2,
    };
    assert!(err.to_string().contains("0"));
    assert!(err.to_string().contains("2"));
}
