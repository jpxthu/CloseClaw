//! Tests for PlanState and PlanPhase types.

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
