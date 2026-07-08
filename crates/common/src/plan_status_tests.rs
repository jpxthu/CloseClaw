//! Tests for PlanStatus enum, status transitions, and serde.

use super::*;
use crate::plan_state::{PlanPath, PlanStatus, StatusTransitionError};

// --- PlanStatus basics ---

#[test]
fn test_plan_status_default_is_draft() {
    assert_eq!(PlanStatus::default(), PlanStatus::Draft);
}

#[test]
fn test_plan_status_all_variants() {
    let variants = [
        PlanStatus::Draft,
        PlanStatus::Confirmed,
        PlanStatus::Executing,
        PlanStatus::Paused,
        PlanStatus::Completed,
    ];
    assert_eq!(variants.len(), 5);
}

#[test]
fn test_plan_status_display() {
    assert_eq!(PlanStatus::Draft.to_string(), "draft");
    assert_eq!(PlanStatus::Confirmed.to_string(), "confirmed");
    assert_eq!(PlanStatus::Executing.to_string(), "executing");
    assert_eq!(PlanStatus::Paused.to_string(), "paused");
    assert_eq!(PlanStatus::Completed.to_string(), "completed");
}

#[test]
fn test_plan_status_serde_snake_case() {
    let cases = [
        (PlanStatus::Draft, r#""draft""#),
        (PlanStatus::Confirmed, r#""confirmed""#),
        (PlanStatus::Executing, r#""executing""#),
        (PlanStatus::Paused, r#""paused""#),
        (PlanStatus::Completed, r#""completed""#),
    ];
    for (status, expected_json) in cases {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(
            json, expected_json,
            "status {:?} should serialize to {}",
            status, expected_json
        );
        let deserialized: PlanStatus = serde_json::from_str(expected_json).unwrap();
        assert_eq!(deserialized, status);
    }
}

#[test]
fn test_status_transition_error_display() {
    let err = StatusTransitionError::InvalidTransition {
        from: PlanStatus::Executing,
        to: PlanStatus::Draft,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("Executing"),
        "should mention from status: {msg}"
    );
    assert!(msg.contains("Draft"), "should mention to status: {msg}");
}

// --- Valid transitions ---

#[test]
fn test_status_transition_draft_to_confirmed() {
    let mut state = PlanState::new();
    assert_eq!(state.status, PlanStatus::Draft);
    assert!(state.transition_status(PlanStatus::Confirmed).is_ok());
    assert_eq!(state.status, PlanStatus::Confirmed);
}

#[test]
fn test_status_transition_confirmed_to_executing() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    assert!(state.transition_status(PlanStatus::Executing).is_ok());
    assert_eq!(state.status, PlanStatus::Executing);
}

#[test]
fn test_status_transition_confirmed_to_paused() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    assert!(state.transition_status(PlanStatus::Paused).is_ok());
    assert_eq!(state.status, PlanStatus::Paused);
}

#[test]
fn test_status_transition_executing_to_completed() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    assert!(state.transition_status(PlanStatus::Completed).is_ok());
    assert_eq!(state.status, PlanStatus::Completed);
}

#[test]
fn test_status_transition_executing_to_paused() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    assert!(state.transition_status(PlanStatus::Paused).is_ok());
    assert_eq!(state.status, PlanStatus::Paused);
}

#[test]
fn test_status_transition_paused_to_executing() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Paused).unwrap();
    assert!(state.transition_status(PlanStatus::Executing).is_ok());
    assert_eq!(state.status, PlanStatus::Executing);
}

#[test]
fn test_status_transition_any_to_draft() {
    let cases = [
        PlanStatus::Confirmed,
        PlanStatus::Executing,
        PlanStatus::Paused,
        PlanStatus::Completed,
    ];
    for from in cases {
        let mut state = PlanState::new();
        if from == PlanStatus::Confirmed {
            state.transition_status(PlanStatus::Confirmed).unwrap();
        } else if from == PlanStatus::Executing {
            state.transition_status(PlanStatus::Confirmed).unwrap();
            state.transition_status(PlanStatus::Executing).unwrap();
        } else if from == PlanStatus::Paused {
            state.transition_status(PlanStatus::Confirmed).unwrap();
            state.transition_status(PlanStatus::Executing).unwrap();
            state.transition_status(PlanStatus::Paused).unwrap();
        } else if from == PlanStatus::Completed {
            state.transition_status(PlanStatus::Confirmed).unwrap();
            state.transition_status(PlanStatus::Executing).unwrap();
            state.transition_status(PlanStatus::Completed).unwrap();
        }
        assert!(
            state.transition_status(PlanStatus::Draft).is_ok(),
            "transition {:?} -> Draft should be valid",
            from
        );
        assert_eq!(state.status, PlanStatus::Draft);
    }
}

// --- Invalid transitions ---

#[test]
fn test_status_transition_draft_to_draft_rejected() {
    let mut state = PlanState::new();
    let err = state.transition_status(PlanStatus::Draft);
    assert!(err.is_err());
    assert_eq!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Draft,
            to: PlanStatus::Draft,
        }
    );
}

#[test]
fn test_status_transition_draft_to_executing_rejected() {
    let mut state = PlanState::new();
    let err = state.transition_status(PlanStatus::Executing);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Draft,
            to: PlanStatus::Executing
        }
    ));
}

#[test]
fn test_status_transition_draft_to_paused_rejected() {
    let mut state = PlanState::new();
    let err = state.transition_status(PlanStatus::Paused);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Draft,
            to: PlanStatus::Paused
        }
    ));
}

#[test]
fn test_status_transition_draft_to_completed_rejected() {
    let mut state = PlanState::new();
    let err = state.transition_status(PlanStatus::Completed);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Draft,
            to: PlanStatus::Completed
        }
    ));
}

#[test]
fn test_status_transition_executing_to_executing_self_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    let err = state.transition_status(PlanStatus::Executing);
    assert!(err.is_err(), "self-transition should be rejected");
}

#[test]
fn test_status_transition_confirmed_to_confirmed_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    let err = state.transition_status(PlanStatus::Confirmed);
    assert!(err.is_err(), "self-transition should be rejected");
}

#[test]
fn test_status_transition_executing_to_confirmed_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    let err = state.transition_status(PlanStatus::Confirmed);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Executing,
            to: PlanStatus::Confirmed
        }
    ));
}

#[test]
fn test_status_transition_paused_to_confirmed_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Paused).unwrap();
    let err = state.transition_status(PlanStatus::Confirmed);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Paused,
            to: PlanStatus::Confirmed
        }
    ));
}

#[test]
fn test_status_transition_completed_to_executing_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Completed).unwrap();
    let err = state.transition_status(PlanStatus::Executing);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Completed,
            to: PlanStatus::Executing
        }
    ));
}

#[test]
fn test_status_transition_completed_to_confirmed_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Completed).unwrap();
    let err = state.transition_status(PlanStatus::Confirmed);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Completed,
            to: PlanStatus::Confirmed
        }
    ));
}

#[test]
fn test_status_transition_completed_to_paused_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Completed).unwrap();
    let err = state.transition_status(PlanStatus::Paused);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Completed,
            to: PlanStatus::Paused
        }
    ));
}

#[test]
fn test_status_transition_paused_to_completed_rejected() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Paused).unwrap();
    let err = state.transition_status(PlanStatus::Completed);
    assert!(err.is_err());
    assert!(matches!(
        err.unwrap_err(),
        StatusTransitionError::InvalidTransition {
            from: PlanStatus::Paused,
            to: PlanStatus::Completed
        }
    ));
}

// --- Serde in PlanState ---

#[test]
fn test_plan_state_serde_with_status_and_path() {
    let state = PlanState {
        status: PlanStatus::Executing,
        explicit_path: Some(PlanPath::Standard),
        phase: PlanPhase::FinalPlan,
        plan_file_path: "/tmp/test.md".into(),
        ..PlanState::default()
    };
    let json = serde_json::to_string(&state).unwrap();
    let deserialized: PlanState = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.status, PlanStatus::Executing);
    assert_eq!(deserialized.explicit_path, Some(PlanPath::Standard));
    assert_eq!(deserialized.phase, PlanPhase::FinalPlan);
    assert_eq!(deserialized.plan_file_path, "/tmp/test.md");
}

#[test]
fn test_plan_state_serde_backward_compat_without_status() {
    let json = r#"{"phase": "research", "plan_file_path": "/tmp/plan.md"}"#;
    let state: PlanState = serde_json::from_str(json).unwrap();
    assert_eq!(state.status, PlanStatus::Draft);
    assert_eq!(state.explicit_path, None);
}

#[test]
fn test_plan_status_serde_in_plan_state() {
    let state = PlanState {
        status: PlanStatus::Paused,
        ..PlanState::default()
    };
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["status"], "paused");
}

// --- Full lifecycle ---

#[test]
fn test_status_full_lifecycle_draft_to_completed() {
    let mut state = PlanState::new();
    assert_eq!(state.status, PlanStatus::Draft);
    state.transition_status(PlanStatus::Confirmed).unwrap();
    assert_eq!(state.status, PlanStatus::Confirmed);
    state.transition_status(PlanStatus::Executing).unwrap();
    assert_eq!(state.status, PlanStatus::Executing);
    state.transition_status(PlanStatus::Completed).unwrap();
    assert_eq!(state.status, PlanStatus::Completed);
}

#[test]
fn test_status_lifecycle_with_pause_resume() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Paused).unwrap();
    assert_eq!(state.status, PlanStatus::Paused);
    state.transition_status(PlanStatus::Executing).unwrap();
    assert_eq!(state.status, PlanStatus::Executing);
    state.transition_status(PlanStatus::Completed).unwrap();
    assert_eq!(state.status, PlanStatus::Completed);
}

#[test]
fn test_status_lifecycle_reject_then_restart() {
    let mut state = PlanState::new();
    state.transition_status(PlanStatus::Confirmed).unwrap();
    state.transition_status(PlanStatus::Executing).unwrap();
    state.transition_status(PlanStatus::Draft).unwrap();
    assert_eq!(state.status, PlanStatus::Draft);
    state.transition_status(PlanStatus::Confirmed).unwrap();
    assert_eq!(state.status, PlanStatus::Confirmed);
}
