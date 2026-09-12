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
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);

    WorkflowEngine::on_goal_injected(&mut run);
    assert!(WorkflowEngine::on_session_idle(&run));
    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert!(run.paused_reason.is_empty());

    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);
    assert_eq!(run.paused_reason, "验收重试次数耗尽");
    assert_eq!(run.pending_verify, 3);

    WorkflowEngine::on_owner_resolve(&mut run);
    assert_eq!(run.phase, Phase::Verifying);
    assert!(run.paused_reason.is_empty());
    assert_eq!(run.pending_verify, 0);

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
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);

    WorkflowEngine::on_goal_injected(&mut run);
    assert_eq!(run.phase, Phase::Executing);

    assert!(WorkflowEngine::on_session_idle(&run));
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify, 1);
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
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers0 = HashMap::new();
    answers0.insert("ready".into(), serde_yaml::Value::Bool(true));
    let action0 = WorkflowEngine::handle_jump(&mut run, &wf, &answers0).unwrap();
    assert_eq!(action0, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
    assert_eq!(run.step_history.len(), 1);

    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers1 = HashMap::new();
    answers1.insert("done".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Goto(2));
    assert_eq!(run.current_step, 2);
    assert_eq!(run.step_history.len(), 2);

    WorkflowEngine::on_goal_injected(&mut run);
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
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    let mut answers1 = HashMap::new();
    answers1.insert("retry".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
    assert!(run.step_history.is_empty());

    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    let mut answers2 = HashMap::new();
    answers2.insert("retry".into(), serde_yaml::Value::Bool(false));
    let action2 = WorkflowEngine::handle_jump(&mut run, &wf, &answers2).unwrap();
    assert_eq!(action2, crate::definition::JumpAction::Complete);
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_owner_terminate_from_blocked() {
    let wf = goto_to_blockable_workflow();
    let mut run = WorkflowEngine::start(&wf);

    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.current_step, 1);
    WorkflowEngine::on_goal_injected(&mut run);

    let result = WorkflowEngine::handle_verify(&mut run, &wf);
    assert!(result.is_err());
    assert_eq!(run.phase, Phase::Executing);

    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);

    WorkflowEngine::on_owner_terminate(&mut run);
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
    assert_eq!(run.pending_verify, 0);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_e2e_pending_verify_resets_after_jump() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 2;

    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let _ = WorkflowEngine::handle_jump(&mut run, &wf, &answers);
    assert_eq!(run.pending_verify, 0);
}

#[test]
fn test_e2e_pending_verify_resets_after_verify() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 2;

    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.pending_verify, 0);
}

// ===========================================================================
// Enum answer mapping and lifecycle
// ===========================================================================

#[test]
fn test_enum_answer_mapping_via_handler() {
    let wf = enum_jump_workflow();
    let mut run = WorkflowEngine::start(&wf);
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
