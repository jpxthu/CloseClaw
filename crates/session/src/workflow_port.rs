//! Workflow engine port for the session crate.
//!
//! [`WorkflowPort`] is the session-owned abstraction over the workflow
//! state machine. Session code keeps the orchestration (tool-result
//! routing, transcript injection, cleanup ordering), while engine
//! operations, definition loading, message rendering, and context-append
//! construction are delegated to this port.
//!
//! The composition root (daemon) supplies the production implementation;
//! tests may supply a thin wrapper around the real engine. All state
//! crosses the port as `serde_json::Value` (the serialized workflow-run
//! state), keeping workflow-specific types out of session production code.

use std::path::Path;

/// Execution phases of a workflow step (session-owned mirror).
///
/// The port translates the engine's internal phase into this mirror so
/// session orchestration can branch on the phase without holding
/// workflow crate types. Debug formatting matches the engine's phase
/// names (`Executing`, `Verifying`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPhase {
    /// Agent is executing step content.
    Executing,
    /// Verify checklist injected, waiting for agent response.
    Verifying,
    /// Jump questions injected, waiting for agent answers.
    Jumping,
    /// Blocked waiting for owner intervention.
    Blocked,
    /// Workflow has completed.
    Complete,
}

/// Hint attached to the next goal message (session-owned mirror).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkflowGoalHint {
    /// First-time goal injection (default).
    #[default]
    Normal,
    /// Reexecute re-entry; the goal message includes a re-execution hint.
    Reexecute,
}

/// Minimal view of a step's jump questions for answer mapping.
///
/// Session orchestration maps single-letter enum answers (A, B, C, …)
/// to option values before submitting answers to the engine; this spec
/// carries the question metadata needed for that mapping.
#[derive(Debug, Clone)]
pub struct JumpQuestionSpec {
    /// Unique key used as parameter key in the jump tool call.
    pub id: String,
    /// Answer type: "boolean" or "enum".
    pub question_type: String,
    /// Option values for enum questions (empty for boolean).
    pub options: Vec<String>,
}

/// Minimal view of a serialized workflow run's orchestration-relevant
/// fields (session-owned mirror).
///
/// The port decodes the run state once and hands the fields session
/// code needs for orchestration and notification building.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunInfo {
    /// ID of the workflow being executed.
    pub workflow_id: String,
    /// Name of the workflow definition (used for definition lookup).
    pub definition_name: String,
    /// Version of the workflow definition recorded on the run.
    pub definition_version: String,
    /// Index of the current step (0-based).
    pub current_step: usize,
    /// Current execution phase.
    pub phase: WorkflowPhase,
    /// Hint for the next goal injection (Normal vs Reexecute).
    pub pending_goal_hint: WorkflowGoalHint,
    /// Reason the workflow is paused while in `Blocked` phase.
    pub paused_reason: String,
    /// Name of the last step-history entry (`None` when history is empty).
    pub last_history_step_name: Option<String>,
}

/// Error returned when a stored workflow-run value cannot be decoded
/// through the [`WorkflowPort`].
///
/// Carries no payload: the port reports decode failure only as `None`.
/// Callers own the failure logging (target semantics stay with the
/// caller's module — mirrors `rebuild_workflow_context_append`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowRunDecodeError;

/// Session-owned port over the workflow state machine engine.
///
/// Method signatures intentionally use only `serde_json::Value` /
/// `String` / `Vec<String>` / session-owned types so session production
/// code never holds workflow crate types. Implementations must be
/// stateless with respect to any single session: all run state is passed
/// in and out as `Value`.
pub trait WorkflowPort: Send + Sync {
    // ── ① Engine operations (state in/out as `Value`) ───────────────

    /// Read the phase of a serialized workflow run.
    ///
    /// Returns `None` when the value is not a decodable workflow run.
    fn run_phase(&self, run: &serde_json::Value) -> Option<WorkflowPhase>;

    /// Read the orchestration-relevant fields of a serialized run.
    ///
    /// Returns `None` when the value is not a decodable workflow run.
    fn run_info(&self, run: &serde_json::Value) -> Option<WorkflowRunInfo>;

    /// Callback after a goal message has been injected into the session.
    ///
    /// Returns the updated run state; returns the input unchanged when
    /// the state is not decodable.
    fn on_goal_injected(&self, run: serde_json::Value) -> serde_json::Value;

    /// Callback after a verify message has been injected.
    ///
    /// Increments the verify counter and transitions to `Verifying`, or
    /// to `Blocked` when the retry limit is reached.
    fn on_verify_injected(
        &self,
        run: serde_json::Value,
        verify_retry_limit: usize,
    ) -> serde_json::Value;

    /// Handle an owner resolve response: transitions a blocked workflow
    /// back to `Verifying` and clears the pause reason.
    fn on_owner_resolve(&self, run: serde_json::Value) -> serde_json::Value;

    /// Handle an owner terminate response: transitions the workflow to
    /// `Complete`.
    fn on_owner_terminate(&self, run: serde_json::Value) -> serde_json::Value;

    /// Transition a run to `Blocked` with the given pause reason
    /// (recovery definition-change path).
    fn mark_blocked(&self, run: serde_json::Value, reason: &str) -> serde_json::Value;

    /// Returns `true` when session idle should trigger a verify injection
    /// (Executing / Verifying / Jumping phases).
    fn on_session_idle(&self, run: &serde_json::Value) -> bool;

    /// Handle a `workflow_verify` tool result.
    ///
    /// Evaluates the verify against the current step and advances the
    /// state machine. Returns the updated run and the resulting phase.
    fn handle_verify(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
    ) -> Result<(serde_json::Value, WorkflowPhase), String>;

    /// Handle a `workflow_jump` tool result.
    ///
    /// Evaluates the answers against the step transitions and executes
    /// the matched action. Returns the updated run and the resulting
    /// phase.
    fn handle_jump(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
        answers: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(serde_json::Value, WorkflowPhase), String>;

    /// Handle a `workflow_blocked` tool result.
    ///
    /// Transitions the workflow to `Blocked` with the given reason.
    /// Returns an error when blocking is not allowed.
    fn handle_blocked(
        &self,
        run: serde_json::Value,
        definition: &serde_json::Value,
        allow_blocked: bool,
        reason: &str,
    ) -> Result<serde_json::Value, String>;

    // ── ② Definition loading ────────────────────────────────────────

    /// Load a workflow definition by name via the three-level priority
    /// lookup (agent workspace → global directory → built-in registry).
    ///
    /// Returns the definition as a serialized `Value` usable with the
    /// other port methods.
    fn load_definition(
        &self,
        name: &str,
        agent_workspace: Option<&Path>,
        global_workflows: Option<&Path>,
    ) -> Result<serde_json::Value, String>;

    // ── ③ Message construction ──────────────────────────────────────

    /// Render the goal message for a step.
    fn goal_message(
        &self,
        definition: &serde_json::Value,
        step: usize,
        hint: WorkflowGoalHint,
    ) -> Option<String>;

    /// Render the jump question message for a step.
    fn jump_message(&self, definition: &serde_json::Value, step: usize) -> Option<String>;

    /// Render the verify checklist message for a step.
    fn verify_message(
        &self,
        definition: &serde_json::Value,
        step: usize,
        allow_blocked: bool,
    ) -> Option<String>;

    // ── ④ Context append construction ───────────────────────────────

    /// Build the workflow context string for `system_injection_appends`.
    fn build_context_append(&self, definition: &serde_json::Value) -> String;

    // ── ⑤ Recovery helper ───────────────────────────────────────────

    /// Build the jump question message for a recovery scenario.
    ///
    /// Returns `None` unless the run is in the `Jumping` phase and the
    /// current step exists in the definition.
    fn recovery_jump_message(
        &self,
        run: &serde_json::Value,
        definition: &serde_json::Value,
    ) -> Option<String>;

    // ── Definition metadata queries ─────────────────────────────────

    /// Returns the verify retry limit declared by the definition.
    fn definition_verify_retry_limit(&self, definition: &serde_json::Value) -> Option<usize>;

    /// Returns the version string declared by the definition, if any.
    fn definition_version(&self, definition: &serde_json::Value) -> Option<String>;

    /// Returns the number of steps in the definition (0 when undecodable).
    fn definition_step_count(&self, definition: &serde_json::Value) -> usize;

    /// Returns the effective `allow_blocked` for a step (step override
    /// falling back to the definition default), or `None` when the step
    /// does not exist.
    fn step_effective_allow_blocked(
        &self,
        definition: &serde_json::Value,
        step: usize,
    ) -> Option<bool>;

    /// Returns the name of a step, or `None` when it does not exist.
    fn step_name(&self, definition: &serde_json::Value, step: usize) -> Option<String>;

    /// Returns the jump questions of a step (empty when the step has
    /// none or does not exist).
    fn step_jump_questions(
        &self,
        definition: &serde_json::Value,
        step: usize,
    ) -> Vec<JumpQuestionSpec>;
}

/// Test-support thin port implementation wrapping the real workflow
/// engine, loader, and definition functions.
///
/// Only compiled for the session crate's own tests; other crates
/// provide their own wrappers (gateway test support) or the production
/// implementation (daemon, wired in a later step).
#[cfg(test)]
pub(crate) mod real_engine_port {
    use std::collections::HashMap;
    use std::path::Path;

    use super::{JumpQuestionSpec, WorkflowGoalHint, WorkflowPhase, WorkflowPort, WorkflowRunInfo};

    use closeclaw_workflow::context_append::build_workflow_context_append;
    use closeclaw_workflow::definition::{
        build_goal_message, build_jump_message, build_verify_message, Workflow,
    };
    use closeclaw_workflow::definition_loader::WorkflowDefinitionLoader;
    use closeclaw_workflow::engine::WorkflowEngine;
    use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

    /// Thin wrapper around the real workflow engine.
    pub(crate) struct RealEngineWorkflowPort;

    fn phase_to_mirror(phase: &Phase) -> WorkflowPhase {
        match phase {
            Phase::Executing => WorkflowPhase::Executing,
            Phase::Verifying => WorkflowPhase::Verifying,
            Phase::Jumping => WorkflowPhase::Jumping,
            Phase::Blocked => WorkflowPhase::Blocked,
            Phase::Complete => WorkflowPhase::Complete,
        }
    }

    fn hint_to_mirror(hint: &GoalHint) -> WorkflowGoalHint {
        match hint {
            GoalHint::Normal => WorkflowGoalHint::Normal,
            GoalHint::Reexecute => WorkflowGoalHint::Reexecute,
        }
    }

    fn hint_from_mirror(hint: WorkflowGoalHint) -> GoalHint {
        match hint {
            WorkflowGoalHint::Normal => GoalHint::Normal,
            WorkflowGoalHint::Reexecute => GoalHint::Reexecute,
        }
    }

    fn decode_run(run: &serde_json::Value) -> Option<WorkflowRun> {
        serde_json::from_value(run.clone()).ok()
    }

    fn decode_definition(definition: &serde_json::Value) -> Option<Workflow> {
        serde_json::from_value(definition.clone()).ok()
    }

    fn encode_run(run: &WorkflowRun) -> serde_json::Value {
        serde_json::to_value(run).unwrap_or(serde_json::Value::Null)
    }

    /// Convert a JSON answer map into the engine's YAML answer map,
    /// mirroring the historical conversion (`to_string` → YAML parse).
    fn json_answers_to_yaml(
        answers: &serde_json::Map<String, serde_json::Value>,
    ) -> HashMap<String, serde_yaml::Value> {
        answers
            .iter()
            .filter_map(|(k, v)| {
                let yaml_val: serde_yaml::Value = serde_yaml::from_str(&v.to_string()).ok()?;
                Some((k.clone(), yaml_val))
            })
            .collect()
    }

    impl WorkflowPort for RealEngineWorkflowPort {
        fn run_phase(&self, run: &serde_json::Value) -> Option<WorkflowPhase> {
            decode_run(run).map(|r| phase_to_mirror(&r.phase))
        }

        fn run_info(&self, run: &serde_json::Value) -> Option<WorkflowRunInfo> {
            let r = decode_run(run)?;
            Some(WorkflowRunInfo {
                workflow_id: r.workflow_id,
                definition_name: r.definition_name,
                definition_version: r.definition_version,
                current_step: r.current_step,
                phase: phase_to_mirror(&r.phase),
                pending_goal_hint: hint_to_mirror(&r.pending_goal_hint),
                paused_reason: r.paused_reason,
                last_history_step_name: r.step_history.last().map(|e| e.step_name.clone()),
            })
        }

        fn on_goal_injected(&self, run: serde_json::Value) -> serde_json::Value {
            match decode_run(&run) {
                Some(mut r) => {
                    WorkflowEngine::on_goal_injected(&mut r);
                    encode_run(&r)
                }
                None => run,
            }
        }

        fn on_verify_injected(
            &self,
            run: serde_json::Value,
            verify_retry_limit: usize,
        ) -> serde_json::Value {
            match decode_run(&run) {
                Some(mut r) => {
                    WorkflowEngine::on_verify_injected(&mut r, verify_retry_limit);
                    encode_run(&r)
                }
                None => run,
            }
        }

        fn on_owner_resolve(&self, run: serde_json::Value) -> serde_json::Value {
            match decode_run(&run) {
                Some(mut r) => {
                    WorkflowEngine::on_owner_resolve(&mut r);
                    encode_run(&r)
                }
                None => run,
            }
        }

        fn on_owner_terminate(&self, run: serde_json::Value) -> serde_json::Value {
            match decode_run(&run) {
                Some(mut r) => {
                    WorkflowEngine::on_owner_terminate(&mut r);
                    encode_run(&r)
                }
                None => run,
            }
        }

        fn mark_blocked(&self, run: serde_json::Value, reason: &str) -> serde_json::Value {
            match decode_run(&run) {
                Some(mut r) => {
                    r.phase = Phase::Blocked;
                    r.paused_reason = reason.to_string();
                    encode_run(&r)
                }
                None => run,
            }
        }

        fn on_session_idle(&self, run: &serde_json::Value) -> bool {
            decode_run(run).is_some_and(|r| WorkflowEngine::on_session_idle(&r))
        }

        fn handle_verify(
            &self,
            run: serde_json::Value,
            definition: &serde_json::Value,
        ) -> Result<(serde_json::Value, WorkflowPhase), String> {
            let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
            WorkflowEngine::handle_verify(&mut r, &def).map_err(|e| e.to_string())?;
            let phase = phase_to_mirror(&r.phase);
            Ok((encode_run(&r), phase))
        }

        fn handle_jump(
            &self,
            run: serde_json::Value,
            definition: &serde_json::Value,
            answers: &serde_json::Map<String, serde_json::Value>,
        ) -> Result<(serde_json::Value, WorkflowPhase), String> {
            let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
            let yaml_answers = json_answers_to_yaml(answers);
            WorkflowEngine::handle_jump(&mut r, &def, &yaml_answers).map_err(|e| e.to_string())?;
            let phase = phase_to_mirror(&r.phase);
            Ok((encode_run(&r), phase))
        }

        fn handle_blocked(
            &self,
            run: serde_json::Value,
            definition: &serde_json::Value,
            allow_blocked: bool,
            reason: &str,
        ) -> Result<serde_json::Value, String> {
            let mut r = decode_run(&run).ok_or_else(|| "undecodable run state".to_string())?;
            let def = decode_definition(definition)
                .ok_or_else(|| "undecodable definition".to_string())?;
            WorkflowEngine::handle_blocked(&mut r, &def, allow_blocked, reason)
                .map_err(|e| e.to_string())?;
            Ok(encode_run(&r))
        }

        fn load_definition(
            &self,
            name: &str,
            agent_workspace: Option<&Path>,
            global_workflows: Option<&Path>,
        ) -> Result<serde_json::Value, String> {
            WorkflowDefinitionLoader::load(name, agent_workspace, global_workflows)
                .map(|def| serde_json::to_value(&def).unwrap_or(serde_json::Value::Null))
                .map_err(|e| e.to_string())
        }

        fn goal_message(
            &self,
            definition: &serde_json::Value,
            step: usize,
            hint: WorkflowGoalHint,
        ) -> Option<String> {
            decode_definition(definition).and_then(|def| {
                def.steps
                    .get(step)
                    .map(|s| build_goal_message(s, hint_from_mirror(hint)))
            })
        }

        fn jump_message(&self, definition: &serde_json::Value, step: usize) -> Option<String> {
            decode_definition(definition)
                .and_then(|def| def.steps.get(step).map(build_jump_message))
        }

        fn verify_message(
            &self,
            definition: &serde_json::Value,
            step: usize,
            allow_blocked: bool,
        ) -> Option<String> {
            decode_definition(definition).and_then(|def| {
                def.steps
                    .get(step)
                    .map(|s| build_verify_message(s, allow_blocked))
            })
        }

        fn build_context_append(&self, definition: &serde_json::Value) -> String {
            match decode_definition(definition) {
                Some(def) => build_workflow_context_append(&def),
                None => String::new(),
            }
        }

        fn recovery_jump_message(
            &self,
            run: &serde_json::Value,
            definition: &serde_json::Value,
        ) -> Option<String> {
            let r = decode_run(run)?;
            let def = decode_definition(definition)?;
            WorkflowEngine::build_recovery_jump_message(&r, &def)
        }

        fn definition_verify_retry_limit(&self, definition: &serde_json::Value) -> Option<usize> {
            decode_definition(definition).map(|def| def.verify_retry_limit)
        }

        fn definition_version(&self, definition: &serde_json::Value) -> Option<String> {
            decode_definition(definition).and_then(|def| def.version)
        }

        fn definition_step_count(&self, definition: &serde_json::Value) -> usize {
            decode_definition(definition).map_or(0, |def| def.steps.len())
        }

        fn step_effective_allow_blocked(
            &self,
            definition: &serde_json::Value,
            step: usize,
        ) -> Option<bool> {
            decode_definition(definition).and_then(|def| {
                def.steps
                    .get(step)
                    .map(|s| s.allow_blocked.unwrap_or(def.allow_blocked))
            })
        }

        fn step_name(&self, definition: &serde_json::Value, step: usize) -> Option<String> {
            decode_definition(definition)
                .and_then(|def| def.steps.get(step).map(|s| s.name.clone()))
        }

        fn step_jump_questions(
            &self,
            definition: &serde_json::Value,
            step: usize,
        ) -> Vec<JumpQuestionSpec> {
            decode_definition(definition)
                .and_then(|def| def.steps.get(step).cloned())
                .map(|s| {
                    s.jump
                        .iter()
                        .map(|q| JumpQuestionSpec {
                            id: q.id.clone(),
                            question_type: q.question_type.clone(),
                            options: q.options.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
    }

    /// Build a test port as `Arc<dyn WorkflowPort>`.
    pub(crate) fn test_port() -> std::sync::Arc<dyn WorkflowPort> {
        std::sync::Arc::new(RealEngineWorkflowPort)
    }
}
