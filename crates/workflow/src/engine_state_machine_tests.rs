//! Unit tests for workflow engine state machine.

use std::collections::HashMap;

use crate::engine::{VerifyAction, WorkflowEngine};
use crate::run::{GoalHint, Phase};
use crate::test_fixtures::*;

// ===========================================================================
// State machine tests
// ===========================================================================

// ---------------------------------------------------------------------------
// start()
// ---------------------------------------------------------------------------

#[test]
fn test_start_initializes_correctly() {
    let wf = simple_workflow();
    let run = WorkflowEngine::start(&wf);
    assert_eq!(run.workflow_id, "simple");
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
    assert!(run.step_history.is_empty());
    assert!(run.step_data.is_null());
    assert_eq!(run.pending_verify, 0);
}

#[test]
fn test_start_sets_version() {
    let wf = simple_workflow();
    let run = WorkflowEngine::start(&wf);
    assert_eq!(run.definition_version, "0.1");
}

// ---------------------------------------------------------------------------
// on_goal_injected()
// ---------------------------------------------------------------------------

#[test]
fn test_on_goal_injected_preserves_step_data_and_resets_hint() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.step_data = serde_yaml::Value::String("old".into());
    run.pending_goal_hint = GoalHint::Reexecute;
    WorkflowEngine::on_goal_injected(&mut run);
    assert_eq!(run.step_data, serde_yaml::Value::String("old".into()));
    assert_eq!(run.pending_goal_hint, GoalHint::Normal);
}

// ---------------------------------------------------------------------------
// on_session_idle()
// ---------------------------------------------------------------------------

#[test]
fn test_on_session_idle_returns_true_when_executing() {
    let wf = simple_workflow();
    let run = WorkflowEngine::start(&wf);
    assert!(WorkflowEngine::on_session_idle(&run));
}

#[test]
fn test_on_session_idle_returns_false_when_jumping() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    assert!(!WorkflowEngine::on_session_idle(&run));
}

#[test]
fn test_on_session_idle_returns_false_when_blocked() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Blocked;
    assert!(!WorkflowEngine::on_session_idle(&run));
}

#[test]
fn test_on_session_idle_returns_false_when_complete() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Complete;
    assert!(!WorkflowEngine::on_session_idle(&run));
}

#[test]
fn test_on_session_idle_returns_true_when_verifying() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Verifying;
    assert!(WorkflowEngine::on_session_idle(&run));
}

// ---------------------------------------------------------------------------
// on_verify_injected() — pending_verify boundaries
// ---------------------------------------------------------------------------

#[test]
fn test_on_verify_injected_increments_count() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify, 1);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_on_verify_injected_twice() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify, 2);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_on_verify_injected_two_times_stays_verifying() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    for _ in 0..2 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.pending_verify, 2);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_on_verify_injected_three_times_enters_blocked() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.pending_verify, 3);
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "验收重试次数耗尽");
}

#[test]
fn test_on_verify_injected_beyond_limit_stays_blocked() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.pending_verify, 4);
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "验收重试次数耗尽");
}

#[test]
fn test_on_verify_injected_custom_limit() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 5);
    }
    assert_eq!(run.pending_verify, 4);
    assert_eq!(run.phase, Phase::Verifying);
    WorkflowEngine::on_verify_injected(&mut run, 5);
    assert_eq!(run.pending_verify, 5);
    assert_eq!(run.phase, Phase::Blocked);
}

// ---------------------------------------------------------------------------
// handle_verify()
// ---------------------------------------------------------------------------

#[test]
fn test_handle_verify_resets_pending_count() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 2;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf);
    assert_eq!(run.pending_verify, 0);
}

#[test]
fn test_handle_verify_with_jumps_enters_jumping() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let action = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action, VerifyAction::Jump);
    assert_eq!(run.phase, Phase::Jumping);
}

#[test]
fn test_handle_verify_no_jumps_default_transition() {
    let wf = two_step_default_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let action = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action, VerifyAction::Jump);
    assert_eq!(run.current_step, 1);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.step_history.len(), 1);
}

#[test]
fn test_handle_verify_no_jumps_no_transitions_returns_error() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::WorkflowError::NoMatchingTransition
    ));
    assert_eq!(run.phase, Phase::Executing);
}

// ---------------------------------------------------------------------------
// handle_jump() — goto
// ---------------------------------------------------------------------------

#[test]
fn test_handle_jump_goto_clears_step_data() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.step_data = serde_yaml::Value::String("data".into());
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert!(run.step_data.is_null());
}

#[test]
fn test_handle_jump_goto_appends_history() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.step_history.len(), 1);
    assert_eq!(run.step_history[0].step_id, 0);
    assert_eq!(run.step_history[0].step_name, "First");
}

#[test]
fn test_handle_jump_goto_sets_phase_executing() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.phase, Phase::Executing);
}

#[test]
fn test_handle_jump_goto_resets_pending_verify() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 2;
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.pending_verify, 0);
}

// ---------------------------------------------------------------------------
// handle_jump() — reexecute
// ---------------------------------------------------------------------------

#[test]
fn test_handle_jump_reexecute_preserves_step_data() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.step_data = serde_yaml::Value::String("keep".into());
    let mut answers = HashMap::new();
    answers.insert("retry".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(run.step_data.as_str().unwrap(), "keep");
}

#[test]
fn test_handle_jump_reexecute_no_history_append() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let mut answers = HashMap::new();
    answers.insert("retry".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert!(run.step_history.is_empty());
}

#[test]
fn test_handle_jump_reexecute_stays_same_step() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let mut answers = HashMap::new();
    answers.insert("retry".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
}

// ---------------------------------------------------------------------------
// handle_jump() — complete
// ---------------------------------------------------------------------------

#[test]
fn test_handle_jump_complete() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(false));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Complete);
    assert_eq!(run.phase, Phase::Complete);
}

// ---------------------------------------------------------------------------
// handle_jump() — no matching transition
// ---------------------------------------------------------------------------

#[test]
fn test_handle_jump_no_match_returns_error() {
    let wf = conditional_only_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let mut answers = HashMap::new();
    answers.insert(
        "go_next".into(),
        serde_yaml::Value::String("neither".into()),
    );
    let result = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::WorkflowError::NoMatchingTransition
    ));
}

// ---------------------------------------------------------------------------
// handle_blocked()
// ---------------------------------------------------------------------------

#[test]
fn test_handle_blocked_allowed() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    WorkflowEngine::handle_blocked(&mut run, &wf, false, "test reason").unwrap();
    assert_eq!(run.phase, Phase::Blocked);
}

#[test]
fn test_handle_blocked_not_allowed_returns_error() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.current_step = 1;
    let result = WorkflowEngine::handle_blocked(&mut run, &wf, false, "test reason");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::WorkflowError::BlockingNotAllowed
    ));
}

#[test]
fn test_handle_blocked_not_allowed_preserves_paused_reason() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.paused_reason = "existing reason".to_string();
    run.current_step = 1;
    let result = WorkflowEngine::handle_blocked(&mut run, &wf, false, "new reason");
    assert!(result.is_err());
    assert_eq!(run.paused_reason, "existing reason");
    assert_eq!(run.phase, Phase::Executing);
}

#[test]
fn test_handle_blocked_uses_workflow_level_when_no_override() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.current_step = 1;
    let result = WorkflowEngine::handle_blocked(&mut run, &wf, false, "test reason");
    assert!(result.is_err());
    let result2 = WorkflowEngine::handle_blocked(&mut run, &wf, true, "test reason");
    assert!(result2.is_ok());
    assert_eq!(run.phase, Phase::Blocked);
}

// ---------------------------------------------------------------------------
// on_owner_resolve()
// ---------------------------------------------------------------------------

#[test]
fn test_on_owner_resolve_resets_pending_and_sets_verifying() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 5;
    run.phase = Phase::Blocked;
    WorkflowEngine::on_owner_resolve(&mut run);
    assert_eq!(run.pending_verify, 0);
    assert_eq!(run.phase, Phase::Verifying);
}

// ---------------------------------------------------------------------------
// on_owner_terminate()
// ---------------------------------------------------------------------------

#[test]
fn test_on_owner_terminate_sets_complete() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    WorkflowEngine::on_owner_terminate(&mut run);
    assert_eq!(run.phase, Phase::Complete);
}

// ---------------------------------------------------------------------------
// paused_reason transitions
// ---------------------------------------------------------------------------

#[test]
fn test_on_verify_injected_sets_paused_reason_when_blocked() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert!(run.paused_reason.is_empty());
    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "验收重试次数耗尽");
}

#[test]
fn test_handle_blocked_sets_paused_reason() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert!(run.paused_reason.is_empty());
    WorkflowEngine::handle_blocked(&mut run, &wf, false, "agent needs help").unwrap();
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "agent needs help");
}

#[test]
fn test_on_owner_resolve_clears_paused_reason() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Blocked;
    run.paused_reason = "验收重试次数耗尽".to_string();
    WorkflowEngine::on_owner_resolve(&mut run);
    assert!(run.paused_reason.is_empty());
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_on_owner_terminate_clears_paused_reason() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Blocked;
    run.paused_reason = "some reason".to_string();
    WorkflowEngine::on_owner_terminate(&mut run);
    assert!(run.paused_reason.is_empty());
    assert_eq!(run.phase, Phase::Complete);
}

#[test]
fn test_paused_reason_stays_empty_through_normal_flow() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert!(run.paused_reason.is_empty());

    WorkflowEngine::on_goal_injected(&mut run);
    assert!(run.paused_reason.is_empty());
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert!(run.paused_reason.is_empty());
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert!(run.paused_reason.is_empty());

    WorkflowEngine::on_goal_injected(&mut run);
    assert!(run.paused_reason.is_empty());
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert!(run.paused_reason.is_empty());
    assert_eq!(run.phase, Phase::Complete);
}

// ---------------------------------------------------------------------------
// is_complete()
// ---------------------------------------------------------------------------

#[test]
fn test_is_complete_true_when_complete() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Complete;
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_is_complete_false_when_executing() {
    let wf = simple_workflow();
    let run = WorkflowEngine::start(&wf);
    assert!(!WorkflowEngine::is_complete(&run));
}
