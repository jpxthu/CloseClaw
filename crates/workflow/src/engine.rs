//! Workflow state machine engine.
//!
//! Manages phase transitions, pending verification counting, and jump
//! evaluation for workflow execution runs.

use std::collections::HashMap;

use crate::definition::{JumpAction, Transition, Workflow};
use crate::error::WorkflowError;
use crate::run::{Phase, WorkflowRun};

/// Action returned by [`WorkflowEngine::handle_verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyAction {
    /// Agent verified; enter jumping phase.
    Jump,
    /// Agent blocked; enter blocked phase.
    Blocked,
}

/// Workflow state machine engine.
///
/// The engine manages [`WorkflowRun`] state transitions. It does not
/// perform any I/O — callers are responsible for injecting messages,
/// evaluating idle states, and persisting runs.
pub struct WorkflowEngine;

impl WorkflowEngine {
    /// Initialize a new [`WorkflowRun`] for the given workflow definition.
    ///
    /// Sets `current_step = 0`, `phase = Executing`, and records the
    /// workflow version as `"0.1"` for newly started runs.
    pub fn start(workflow: &Workflow) -> WorkflowRun {
        WorkflowRun {
            workflow_id: workflow.id.clone(),
            definition_version: "0.1".to_string(),
            current_step: 0,
            phase: Phase::Executing,
            step_history: Vec::new(),
            step_data: serde_yaml::Value::Null,
            pending_verify: 0,
        }
    }

    /// Callback after a goal message has been injected into the session.
    ///
    /// Records the current timestamp so downstream tools can reference
    /// when the current step started executing.
    pub fn on_goal_injected(run: &mut WorkflowRun) {
        run.step_data = serde_yaml::Value::Null;
        tracing::debug!(
            step = run.current_step,
            "goal injected, ready for agent execution"
        );
    }

    /// Called when the session becomes idle.
    ///
    /// Returns `true` if a verify message should be injected (i.e., the
    /// run is in the `Executing` phase). Returns `false` for all other
    /// phases where idle transitions are not relevant.
    pub fn on_session_idle(run: &WorkflowRun) -> bool {
        run.phase == Phase::Executing
    }

    /// Callback after a verify message has been injected.
    ///
    /// Increments `pending_verify`. If the count exceeds the limit
    /// defined in `workflow`, the phase transitions to `Blocked`.
    pub fn on_verify_injected(run: &mut WorkflowRun, verify_retry_limit: usize) {
        run.pending_verify += 1;
        tracing::debug!(
            pending = run.pending_verify,
            limit = verify_retry_limit,
            "verify injected"
        );
        if run.pending_verify > verify_retry_limit {
            run.phase = Phase::Blocked;
            tracing::warn!(
                pending = run.pending_verify,
                limit = verify_retry_limit,
                "verify limit exceeded, entering blocked"
            );
        }
    }

    /// Handle an agent `workflow_verify` call.
    ///
    /// Returns [`VerifyAction::Jump`] if the current step has jump
    /// questions, or [`VerifyAction::Blocked`] if there are no jump
    /// questions and the step cannot proceed.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::NoMatchingTransition`] if the current
    /// step has neither jump questions nor a default transition.
    pub fn handle_verify(
        run: &mut WorkflowRun,
        workflow: &Workflow,
    ) -> Result<VerifyAction, WorkflowError> {
        run.pending_verify = 0;

        let step = workflow
            .steps
            .get(run.current_step)
            .ok_or(WorkflowError::StepNotFound(run.current_step))?;

        if !step.jump.is_empty() {
            run.phase = Phase::Jumping;
            tracing::debug!(step = run.current_step, "verify passed, entering jumping");
            return Ok(VerifyAction::Jump);
        }

        // No jump questions — attempt to find a default transition.
        let answers: HashMap<String, serde_yaml::Value> = HashMap::new();
        if let Some((action, _target)) = evaluate_transitions(&step.transitions, &answers) {
            match action {
                JumpAction::Goto(target) => {
                    Self::execute_goto(run, workflow, target)?;
                    return Ok(VerifyAction::Jump);
                }
                JumpAction::Reexecute(target) => {
                    Self::execute_reexecute(run, workflow, target)?;
                    return Ok(VerifyAction::Jump);
                }
                JumpAction::Complete => {
                    run.phase = Phase::Complete;
                    tracing::debug!("workflow complete after verify");
                    return Ok(VerifyAction::Jump);
                }
            }
        }

        // No transitions matched and no jump — block.
        run.phase = Phase::Blocked;
        tracing::debug!(
            step = run.current_step,
            "no transitions matched, entering blocked"
        );
        Ok(VerifyAction::Blocked)
    }

    /// Handle an agent `workflow_jump` call.
    ///
    /// Evaluates the provided answers against the current step's
    /// transitions, executes the matched action (goto/reexecute/complete),
    /// and updates the run state accordingly.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::NoMatchingTransition`] if no transition
    /// matches and no default exists, or [`WorkflowError::StepNotFound`]
    /// if the target step does not exist.
    pub fn handle_jump(
        run: &mut WorkflowRun,
        workflow: &Workflow,
        answers: &HashMap<String, serde_yaml::Value>,
    ) -> Result<JumpAction, WorkflowError> {
        let step = workflow
            .steps
            .get(run.current_step)
            .ok_or(WorkflowError::StepNotFound(run.current_step))?;

        let (action, _target) = evaluate_transitions(&step.transitions, answers)
            .ok_or(WorkflowError::NoMatchingTransition)?;

        match action {
            JumpAction::Goto(t) => {
                Self::execute_goto(run, workflow, t)?;
            }
            JumpAction::Reexecute(t) => {
                Self::execute_reexecute(run, workflow, t)?;
            }
            JumpAction::Complete => {
                run.phase = Phase::Complete;
                tracing::debug!("workflow complete after jump");
            }
        }

        Ok(action)
    }

    /// Handle an agent `workflow_blocked` call.
    ///
    /// Sets the phase to `Blocked`. Returns an error if blocking is not
    /// allowed for the current step.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::BlockingNotAllowed`] if the step's
    /// `allow_blocked` is `false`.
    pub fn handle_blocked(
        run: &mut WorkflowRun,
        workflow: &Workflow,
        allow_blocked: bool,
    ) -> Result<(), WorkflowError> {
        let step = workflow
            .steps
            .get(run.current_step)
            .ok_or(WorkflowError::StepNotFound(run.current_step))?;

        let effective = step.allow_blocked.unwrap_or(allow_blocked);
        if !effective {
            return Err(WorkflowError::BlockingNotAllowed);
        }

        run.phase = Phase::Blocked;
        tracing::debug!(
            step = run.current_step,
            "agent called blocked, entering blocked"
        );
        Ok(())
    }

    /// Called when the owner resolves a blocked workflow.
    ///
    /// Resets `pending_verify` to zero and transitions the phase back
    /// to `Verifying`.
    pub fn on_owner_resolve(run: &mut WorkflowRun) {
        run.pending_verify = 0;
        run.phase = Phase::Verifying;
        tracing::debug!("owner resolved blocked, entering verifying");
    }

    /// Called when the owner terminates the workflow.
    ///
    /// Sets the phase to `Complete`.
    pub fn on_owner_terminate(run: &mut WorkflowRun) {
        run.phase = Phase::Complete;
        tracing::debug!("owner terminated workflow");
    }

    /// Check whether the workflow run has reached the `Complete` phase.
    pub fn is_complete(run: &WorkflowRun) -> bool {
        run.phase == Phase::Complete
    }

    /// Execute a goto action: clear step_data, append history, set phase.
    fn execute_goto(
        run: &mut WorkflowRun,
        workflow: &Workflow,
        target: usize,
    ) -> Result<(), WorkflowError> {
        if workflow.steps.get(target).is_none() {
            return Err(WorkflowError::StepNotFound(target));
        }

        // Append current step to history before moving.
        let step = &workflow.steps[run.current_step];
        run.step_history.push(crate::run::StepHistoryEntry {
            step_id: run.current_step,
            step_name: step.name.clone(),
            completed_at: chrono::Utc::now().to_rfc3339(),
        });

        run.current_step = target;
        run.step_data = serde_yaml::Value::Null;
        run.pending_verify = 0;
        run.phase = Phase::Executing;
        tracing::debug!(target, "goto executed");
        Ok(())
    }

    /// Execute a reexecute action: keep step_data, set phase.
    fn execute_reexecute(
        run: &mut WorkflowRun,
        workflow: &Workflow,
        target: usize,
    ) -> Result<(), WorkflowError> {
        if workflow.steps.get(target).is_none() {
            return Err(WorkflowError::StepNotFound(target));
        }

        run.current_step = target;
        // step_data is preserved (not cleared).
        run.pending_verify = 0;
        run.phase = Phase::Executing;
        tracing::debug!(target, "reexecute executed");
        Ok(())
    }
}

/// Evaluate transition conditions against the provided answers.
///
/// Iterates through `transitions` in order. For each transition with a
/// `when` clause, all conditions must match (AND logic). The first
/// matching transition wins. If no transition matches and a default
/// (no `when`) transition exists, it is used.
///
/// Returns `Some((action, target_step))` for a matched transition,
/// or `None` if no transition matches.
pub fn evaluate_transitions(
    transitions: &[Transition],
    answers: &HashMap<String, serde_yaml::Value>,
) -> Option<(JumpAction, Option<usize>)> {
    let mut default: Option<&Transition> = None;

    for t in transitions {
        match &t.when {
            Some(when_value) => {
                if evaluate_when(when_value, answers) {
                    return Some(to_jump_action(t));
                }
            }
            None => {
                default = Some(t);
            }
        }
    }

    default.map(to_jump_action)
}

/// Convert a [`Transition`] into a [`JumpAction`] pair.
fn to_jump_action(t: &Transition) -> (JumpAction, Option<usize>) {
    let action = match t.action.as_str() {
        "goto" => JumpAction::Goto(t.target_step.unwrap_or(0)),
        "reexecute" => JumpAction::Reexecute(t.target_step.unwrap_or(0)),
        "complete" => JumpAction::Complete,
        _ => JumpAction::Complete,
    };
    (action, t.target_step)
}

/// Evaluate all conditions in a `when` mapping (AND logic).
///
/// Supports:
/// - Boolean values: native YAML bool comparison
/// - String values: direct string comparison
fn evaluate_when(when: &serde_yaml::Value, answers: &HashMap<String, serde_yaml::Value>) -> bool {
    let mapping = match when.as_mapping() {
        Some(m) => m,
        None => return true,
    };

    for (key, expected) in mapping {
        let key_str = match key.as_str() {
            Some(s) => s,
            None => return false,
        };

        let answer = match answers.get(key_str) {
            Some(a) => a,
            None => return false,
        };

        if !values_equal(answer, expected) {
            return false;
        }
    }

    true
}

/// Check if two YAML values are equal.
///
/// For boolean values, uses native YAML bool comparison.
/// For other types, falls back to string comparison.
fn values_equal(answer: &serde_yaml::Value, expected: &serde_yaml::Value) -> bool {
    // Fast path: native YAML equality (handles bool, number, null, etc.)
    if answer == expected {
        return true;
    }

    // Fallback: compare string representations for enum/string values.
    match (answer.as_str(), expected.as_str()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}
