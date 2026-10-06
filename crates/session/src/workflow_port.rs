//! Workflow engine port for the session crate.
//!
//! [`WorkflowPort`] is the session-owned abstraction over the workflow
//! state machine. Session code keeps the orchestration (tool-result
//! routing, transcript injection, cleanup ordering), while engine
//! operations, definition loading, message rendering, and context-append
//! management are delegated to this port.
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
/// workflow crate types.
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

    /// Callback after a goal message has been injected into the session.
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

    // ── ④ Context append build / detect / remove ────────────────────

    /// Build the workflow context string for `system_injection_appends`.
    fn build_context_append(&self, definition: &serde_json::Value) -> String;

    /// Returns `true` when a workflow context marker exists in the list.
    fn has_workflow_context(&self, appends: &[String]) -> bool;

    /// Remove all workflow context markers from the list, returning the
    /// number of removed items.
    fn remove_workflow_context(&self, appends: &mut Vec<String>) -> usize;

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
