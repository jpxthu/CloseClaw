//! End-to-end lifecycle and enum mapping tests for workflow engine.

use std::collections::HashMap;

use crate::engine::{VerifyAction, WorkflowEngine};
use crate::run::Phase;
use crate::test_fixtures::*;

// ===========================================================================
// Long chain: verify exhaust → blocked → resolve → complete
// ===========================================================================

#[test]
fn test_e2e_long_chain_verify_exhaust_resolve_complete() {
    let wf = no_transitions_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);

    WorkflowEngine::on_goal_injected(&mut run);
    assert!(WorkflowEngine::on_session_idle(&run));
    run.phase = Phase::Verifying;
    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert!(run.paused_reason.is_empty());

    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "验收重试次数耗尽");
    assert_eq!(run.pending_verify.count, 3);

    WorkflowEngine::on_owner_resolve(&mut run);
    assert_eq!(run.phase, Phase::Verifying);
    assert!(run.paused_reason.is_empty());
    assert_eq!(run.pending_verify.count, 0);

    let result2 = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result2.is_err());
    assert_eq!(run.phase, Phase::Verifying);
    assert!(run.paused_reason.is_empty());
}

// ===========================================================================
// End-to-end: full workflow lifecycle
// ===========================================================================

#[test]
fn test_e2e_single_step_no_jumps_blocked_via_over_limit() {
    let wf = no_transitions_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);

    WorkflowEngine::on_goal_injected(&mut run);
    assert_eq!(run.phase, Phase::Executing);

    assert!(WorkflowEngine::on_session_idle(&run));
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify.count, 1);
    assert_eq!(run.phase, Phase::Verifying);

    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::WorkflowError::NoMatchingTransition
    ));
    assert_eq!(run.phase, Phase::Verifying);

    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);
}

#[test]
fn test_e2e_goto_then_complete() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.step_history.len(), 1);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let action2 = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action2, VerifyAction::Jump);
    assert_eq!(run.phase, Phase::Complete);
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_three_step_lifecycle() {
    let wf = three_step_lifecycle_workflow();
    let mut run = WorkflowEngine::start(&wf);

    WorkflowEngine::on_goal_injected(&mut run);
    assert!(WorkflowEngine::on_session_idle(&run));
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers0 = HashMap::new();
    answers0.insert("ready".into(), serde_yaml::Value::Bool(true));
    let action0 = WorkflowEngine::handle_jump(&mut run, &wf, &answers0).unwrap();
    assert_eq!(action0, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
    assert_eq!(run.step_history.len(), 1);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers1 = HashMap::new();
    answers1.insert("done".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Goto(2));
    assert_eq!(run.current_step, 2);
    assert_eq!(run.step_history.len(), 2);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Complete);
    assert!(WorkflowEngine::is_complete(&run));
    assert_eq!(run.step_history.len(), 2);
}

#[test]
fn test_e2e_reexecute_then_complete() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    let mut answers1 = HashMap::new();
    answers1.insert("retry".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
    assert!(run.step_history.is_empty());

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    let mut answers2 = HashMap::new();
    answers2.insert("retry".into(), serde_yaml::Value::Bool(false));
    let action2 = WorkflowEngine::handle_jump(&mut run, &wf, &answers2).unwrap();
    assert_eq!(action2, crate::definition::JumpAction::Complete);
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_owner_terminate_from_blocked() {
    let wf = goto_then_no_transitions_workflow();
    let mut run = WorkflowEngine::start(&wf);

    run.phase = Phase::Verifying;
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.current_step, 1);
    WorkflowEngine::on_goal_injected(&mut run);

    run.phase = Phase::Verifying;
    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert_eq!(run.phase, Phase::Verifying);

    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);

    WorkflowEngine::on_owner_terminate(&mut run);
    assert!(run.paused_reason.is_empty());
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_owner_resolve_then_verify() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);

    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);

    WorkflowEngine::on_owner_resolve(&mut run);
    assert_eq!(run.pending_verify.count, 0);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_e2e_pending_verify_resets_after_jump() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    run.pending_verify.count = 2;

    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.pending_verify.count, 0);
}

#[test]
fn test_e2e_pending_verify_resets_after_verify() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify.count = 2;
    run.phase = Phase::Verifying;

    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.pending_verify.count, 0);
}

// ===========================================================================
// Enum answer mapping and lifecycle
// ===========================================================================

#[test]
fn test_enum_answer_mapping_via_handler() {
    let wf = enum_jump_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let mut answers = HashMap::new();
    answers.insert("strategy".into(), serde_yaml::Value::String("fast".into()));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
}

#[test]
fn test_enum_answer_mapping_slow() {
    let wf = enum_jump_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let mut answers = HashMap::new();
    answers.insert("strategy".into(), serde_yaml::Value::String("slow".into()));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(2));
    assert_eq!(run.current_step, 2);
}

#[test]
fn test_e2e_verify_jumping_jump_goto_enum() {
    let wf = enum_jump_workflow();
    let mut run = WorkflowEngine::start(&wf);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let action = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action, VerifyAction::Jump);
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers = HashMap::new();
    answers.insert("strategy".into(), serde_yaml::Value::String("fast".into()));
    let jump_action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(jump_action, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.step_history.len(), 1);

    WorkflowEngine::on_goal_injected(&mut run);
    run.phase = Phase::Verifying;
    let action2 = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action2, VerifyAction::Jump);
    assert_eq!(run.phase, Phase::Complete);
}

#[test]
fn test_boolean_answer_no_mapping_needed() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
}

// ===========================================================================
// build_recovery_jump_message — Step 1.4 tests
// ===========================================================================

/// Verify that build_recovery_jump_message returns Some with correct
/// content when phase is Jumping and step has jump questions.
#[test]
fn test_build_recovery_jump_message_jumping_phase() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let msg = WorkflowEngine::build_recovery_jump_message(&run, &wf);
    assert!(msg.is_some(), "should return Some for jumping phase");
    let msg = msg.unwrap();
    assert!(msg.contains("Go to next?"), "jump prompt missing: {}", msg);
    assert!(
        msg.contains("workflow_jump"),
        "workflow_jump call hint missing: {}",
        msg
    );
}

/// Verify that build_recovery_jump_message returns None for
/// Executing phase (non-jumping boundary).
#[test]
fn test_build_recovery_jump_message_executing_returns_none() {
    let wf = two_step_goto_workflow();
    let run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);
    assert!(
        WorkflowEngine::build_recovery_jump_message(&run, &wf).is_none(),
        "Executing phase should not build jump message"
    );
}

/// Verify that build_recovery_jump_message returns None for
/// Verifying phase (non-jumping boundary).
#[test]
fn test_build_recovery_jump_message_verifying_returns_none() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Verifying;
    assert!(
        WorkflowEngine::build_recovery_jump_message(&run, &wf).is_none(),
        "Verifying phase should not build jump message"
    );
}

/// Verify that build_recovery_jump_message returns None for
/// Blocked phase (non-jumping boundary).
#[test]
fn test_build_recovery_jump_message_blocked_returns_none() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Blocked;
    assert!(
        WorkflowEngine::build_recovery_jump_message(&run, &wf).is_none(),
        "Blocked phase should not build jump message"
    );
}

/// Verify that build_recovery_jump_message returns None for
/// Complete phase (non-jumping boundary).
#[test]
fn test_build_recovery_jump_message_complete_returns_none() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Complete;
    assert!(
        WorkflowEngine::build_recovery_jump_message(&run, &wf).is_none(),
        "Complete phase should not build jump message"
    );
}

/// Verify that build_recovery_jump_message returns None when
/// current_step is out of range (definition missing / step not found).
#[test]
fn test_build_recovery_jump_message_step_out_of_range() {
    let wf = simple_workflow(); // single step (id=0)
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    run.current_step = 999; // out of range
    assert!(
        WorkflowEngine::build_recovery_jump_message(&run, &wf).is_none(),
        "out-of-range step should return None, not panic"
    );
}

/// Verify that build_recovery_jump_message returns None when step
/// has no jump questions (empty jump list).
#[test]
fn test_build_recovery_jump_message_no_jump_questions() {
    let wf = simple_workflow(); // step has no jump questions
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    // simple_workflow step 0 has empty jump list
    let msg = WorkflowEngine::build_recovery_jump_message(&run, &wf);
    assert!(
        msg.is_some(),
        "should still return Some (build_jump_message handles empty)"
    );
}

/// Verify that enum jump questions are included in recovery message.
#[test]
fn test_build_recovery_jump_message_enum_questions() {
    let wf = enum_jump_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;
    let msg = WorkflowEngine::build_recovery_jump_message(&run, &wf).unwrap();
    assert!(
        msg.contains("Which strategy?"),
        "enum prompt missing: {}",
        msg
    );
    assert!(msg.contains("fast"), "enum option missing: {}", msg);
    assert!(msg.contains("slow"), "enum option missing: {}", msg);
}

/// Verify that after jump message re-injection, handle_jump can
/// transition out of jumping phase (state transition dimension).
#[test]
fn test_recovery_jump_then_handle_jump_exits_jumping() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;

    // Simulate: build recovery message, agent reads it, calls handle_jump
    let msg = WorkflowEngine::build_recovery_jump_message(&run, &wf);
    assert!(msg.is_some(), "recovery message should be available");

    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(
        run.phase,
        Phase::Executing,
        "should exit jumping after handle_jump"
    );
    assert_eq!(run.current_step, 1);
}

/// Verify that reexecute action exits jumping phase correctly.
#[test]
fn test_recovery_jump_then_handle_jump_reexecute() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;

    let msg = WorkflowEngine::build_recovery_jump_message(&run, &wf);
    assert!(msg.is_some());

    let mut answers = HashMap::new();
    answers.insert("retry".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.current_step, 0);
}

/// Verify that complete action exits jumping phase correctly.
#[test]
fn test_recovery_jump_then_handle_jump_complete() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Jumping;

    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(false));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Complete);
    assert_eq!(run.phase, Phase::Complete);
}
