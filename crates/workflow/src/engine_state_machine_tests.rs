//! Unit tests for workflow engine state machine and end-to-end lifecycle.

use std::collections::HashMap;

use crate::definition::Workflow;
use crate::engine::{VerifyAction, WorkflowEngine};
use crate::run::Phase;

// ---------------------------------------------------------------------------
// Helpers: workflow fixtures
// ---------------------------------------------------------------------------

fn simple_workflow() -> Workflow {
    let yaml = r#"
id: simple
name: Simple
description: Single step workflow
steps:
  - id: 0
    name: Only Step
    goal: Do the thing
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn two_step_goto_workflow() -> Workflow {
    let yaml = r#"
id: two-step
name: Two Step
description: Two step workflow
steps:
  - id: 0
    name: First
    goal: Step one
    jump:
      - id: go_next
        prompt: Go to next?
        type: boolean
    transitions:
      - when:
          go_next: true
        action: goto
        target_step: 1
      - action: complete
  - id: 1
    name: Second
    goal: Step two
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn two_step_default_goto_workflow() -> Workflow {
    let yaml = r#"
id: default-goto
name: Default Goto
description: Default goto on first step
steps:
  - id: 0
    name: First
    goal: Step one
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Second
    goal: Step two
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn reexecute_workflow() -> Workflow {
    let yaml = r#"
id: reexec
name: Reexec
description: Reexecute workflow
steps:
  - id: 0
    name: Loop
    goal: Loop until done
    jump:
      - id: retry
        prompt: Retry?
        type: boolean
    transitions:
      - when:
          retry: true
        action: reexecute
        target_step: 0
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn blocked_workflow() -> Workflow {
    let yaml = r#"
id: blocked
name: Blocked
description: Blockable workflow
allow_blocked: false
steps:
  - id: 0
    name: Can Block
    allow_blocked: true
    goal: Might block
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Cannot Block
    goal: Must not block
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn three_step_lifecycle_workflow() -> Workflow {
    let yaml = r#"
id: lifecycle
name: Lifecycle
description: Full lifecycle
steps:
  - id: 0
    name: Setup
    goal: Set up
    jump:
      - id: ready
        prompt: Ready?
        type: boolean
    transitions:
      - when:
          ready: true
        action: goto
        target_step: 1
      - action: complete
  - id: 1
    name: Execute
    goal: Do work
    jump:
      - id: done
        prompt: Done?
        type: boolean
    transitions:
      - when:
          done: true
        action: goto
        target_step: 2
      - action: complete
  - id: 2
    name: Cleanup
    goal: Clean up
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn conditional_only_workflow() -> Workflow {
    let yaml = r#"
id: conditional-only
name: Conditional Only
description: No default transitions
steps:
  - id: 0
    name: Decide
    goal: Choose
    jump:
      - id: go_next
        prompt: Go?
        type: boolean
    transitions:
      - when:
          go_next: true
        action: goto
        target_step: 1
  - id: 1
    name: End
    goal: Done
    transitions:
      - action: complete
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

fn goto_to_blockable_workflow() -> Workflow {
    let yaml = r#"
id: goto-block
name: Goto Block
description: Goto then block
steps:
  - id: 0
    name: First
    goal: Go to step 1
    transitions:
      - action: goto
        target_step: 1
  - id: 1
    name: Second
    goal: Will block
"#;
    Workflow::parse_frontmatter(yaml).unwrap()
}

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
fn test_on_goal_injected_clears_step_data() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.step_data = serde_yaml::Value::String("old".into());
    WorkflowEngine::on_goal_injected(&mut run);
    assert!(run.step_data.is_null());
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
fn test_on_session_idle_returns_false_when_verifying() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.phase = Phase::Verifying;
    assert!(!WorkflowEngine::on_session_idle(&run));
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
    assert_eq!(run.phase, Phase::Executing);
}

#[test]
fn test_on_verify_injected_twice() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify, 2);
    assert_eq!(run.phase, Phase::Executing);
}

#[test]
fn test_on_verify_injected_three_times_stays_executing() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    // default verify_retry_limit = 3; 3 > 3 is false
    for _ in 0..3 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.pending_verify, 3);
    assert_eq!(run.phase, Phase::Executing);
}

#[test]
fn test_on_verify_injected_four_times_enters_blocked() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    // default verify_retry_limit = 3; 4 > 3 → blocked
    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.pending_verify, 4);
    assert_eq!(run.phase, Phase::Blocked);
}

#[test]
fn test_on_verify_injected_custom_limit() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    for _ in 0..5 {
        WorkflowEngine::on_verify_injected(&mut run, 5);
    }
    assert_eq!(run.pending_verify, 5);
    assert_eq!(run.phase, Phase::Executing);
    // one more → blocked
    WorkflowEngine::on_verify_injected(&mut run, 5);
    assert_eq!(run.pending_verify, 6);
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
    // step 0 has jump questions → Jump
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
    // default goto → step 1, phase = Executing
    assert_eq!(run.current_step, 1);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.step_history.len(), 1);
}

#[test]
fn test_handle_verify_no_jumps_no_transitions_blocks() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    let action = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action, VerifyAction::Blocked);
    assert_eq!(run.phase, Phase::Blocked);
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
    // step 0 has allow_blocked = true (override)
    WorkflowEngine::handle_blocked(&mut run, &wf, false).unwrap();
    assert_eq!(run.phase, Phase::Blocked);
}

#[test]
fn test_handle_blocked_not_allowed_returns_error() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.current_step = 1; // step 1 has no override, workflow allow_blocked = false
    let result = WorkflowEngine::handle_blocked(&mut run, &wf, false);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        crate::error::WorkflowError::BlockingNotAllowed
    ));
}

#[test]
fn test_handle_blocked_uses_workflow_level_when_no_override() {
    let wf = blocked_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.current_step = 1; // step 1: no override
                          // workflow-level allow_blocked = false → blocked not allowed
    let result = WorkflowEngine::handle_blocked(&mut run, &wf, false);
    assert!(result.is_err());
    // but if workflow-level allow_blocked were true, it would succeed
    let result2 = WorkflowEngine::handle_blocked(&mut run, &wf, true);
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

// ===========================================================================
// End-to-end: full workflow lifecycle
// ===========================================================================

#[test]
fn test_e2e_single_step_no_jumps() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);
    assert_eq!(run.phase, Phase::Executing);

    // Goal injected
    WorkflowEngine::on_goal_injected(&mut run);
    assert_eq!(run.phase, Phase::Executing);

    // Session idle → need verify
    assert!(WorkflowEngine::on_session_idle(&run));
    WorkflowEngine::on_verify_injected(&mut run, wf.verify_retry_limit);
    assert_eq!(run.pending_verify, 1);

    // Handle verify → no jumps, no transitions → blocked
    let action = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action, VerifyAction::Blocked);
    assert_eq!(run.phase, Phase::Blocked);
}

#[test]
fn test_e2e_goto_then_complete() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);

    // Step 0: goal → idle → verify → jumping
    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    // Step 0: jump → goto step 1
    let mut answers = HashMap::new();
    answers.insert("go_next".into(), serde_yaml::Value::Bool(true));
    let action = WorkflowEngine::handle_jump(&mut run, &wf, &answers).unwrap();
    assert_eq!(action, crate::definition::JumpAction::Goto(1));
    assert_eq!(run.current_step, 1);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.step_history.len(), 1);

    // Step 1: goal → idle → verify (no jumps, default complete → Complete)
    WorkflowEngine::on_goal_injected(&mut run);
    let action2 = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(action2, VerifyAction::Jump);
    // Default complete transition on step 1 sets phase directly
    assert_eq!(run.phase, Phase::Complete);
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_three_step_lifecycle() {
    let wf = three_step_lifecycle_workflow();
    let mut run = WorkflowEngine::start(&wf);

    // === Step 0: Setup ===
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

    // === Step 1: Execute ===
    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Jumping);

    let mut answers1 = HashMap::new();
    answers1.insert("done".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Goto(2));
    assert_eq!(run.current_step, 2);
    assert_eq!(run.step_history.len(), 2);

    // === Step 2: Cleanup ===
    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    // Step 2 has default complete transition, no jump questions
    assert_eq!(run.phase, Phase::Complete);
    assert!(WorkflowEngine::is_complete(&run));
    assert_eq!(run.step_history.len(), 2);
}

#[test]
fn test_e2e_reexecute_then_complete() {
    let wf = reexecute_workflow();
    let mut run = WorkflowEngine::start(&wf);

    // First execution: retry = true → reexecute
    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    let mut answers1 = HashMap::new();
    answers1.insert("retry".into(), serde_yaml::Value::Bool(true));
    let action1 = WorkflowEngine::handle_jump(&mut run, &wf, &answers1).unwrap();
    assert_eq!(action1, crate::definition::JumpAction::Reexecute(0));
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
    assert!(run.step_history.is_empty());

    // Second execution: retry = false → complete
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

    // Default goto goes to step 1, then no jumps → blocked
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.current_step, 1);
    WorkflowEngine::on_goal_injected(&mut run);
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.phase, Phase::Blocked);

    // Owner terminates
    WorkflowEngine::on_owner_terminate(&mut run);
    assert!(WorkflowEngine::is_complete(&run));
}

#[test]
fn test_e2e_owner_resolve_then_verify() {
    let wf = simple_workflow();
    let mut run = WorkflowEngine::start(&wf);

    // Trigger blocked via pending_verify overflow
    for _ in 0..4 {
        WorkflowEngine::on_verify_injected(&mut run, 3);
    }
    assert_eq!(run.phase, Phase::Blocked);

    // Owner resolves
    WorkflowEngine::on_owner_resolve(&mut run);
    assert_eq!(run.pending_verify, 0);
    assert_eq!(run.phase, Phase::Verifying);
}

#[test]
fn test_e2e_pending_verify_resets_after_jump() {
    let wf = two_step_goto_workflow();
    let mut run = WorkflowEngine::start(&wf);
    run.pending_verify = 2;

    // Jump resets pending_verify
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

    // handle_verify resets pending_verify
    let _ = WorkflowEngine::handle_verify(&mut run, &wf).unwrap();
    assert_eq!(run.pending_verify, 0);
}
