//! Core types for the execution engine.

use closeclaw_common::{ExecutionStepStatus, PlanState};
use serde::{Deserialize, Serialize};

/// Execution mode — determines how steps are dispatched.
///
/// - `Inline`: Steps run in the parent session directly.
/// - `SpawnPerStep`: Each step spawns an independent sub-agent.
/// - `SpawnAllSteps`: A single sub-agent executes all steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Execute steps inline in the parent session.
    Inline,
    /// Spawn a new sub-agent for each step.
    SpawnPerStep,
    /// Spawn one sub-agent to execute all steps.
    SpawnAllSteps,
}

/// Retry strategy after a step failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryStrategy {
    /// Spawn a fresh sub-agent with clean context.
    Fresh,
    /// Continue in the same sub-agent context, preserving error history.
    Continue,
}

/// Verification trigger policy — when to spawn a verification agent
/// after a step completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyTrigger {
    /// Never verify after any step.
    Never,
    /// Verify only after non-trivial steps.
    NonTrivial,
    /// Always verify after every step.
    Always,
}

/// Execution configuration — controls scheduling behavior, retry, and
/// verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Execution mode (inline, spawn per step, spawn all steps).
    pub mode: ExecutionMode,
    /// Maximum number of retries per failed step.
    pub max_retries: u32,
    /// Strategy for retries (fresh or continue).
    pub retry_strategy: RetryStrategy,
    /// When to trigger step verification.
    pub verify_trigger: VerifyTrigger,
    /// Optional step selection (0-based indices). When `Some`, only the
    /// specified steps are executed. When `None`, all steps run.
    #[serde(default)]
    pub step_selection: Option<Vec<usize>>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Inline,
            max_retries: 3,
            retry_strategy: RetryStrategy::Fresh,
            verify_trigger: VerifyTrigger::NonTrivial,
            step_selection: None,
        }
    }
}

impl From<&PlanState> for ExecutionConfig {
    /// Create an `ExecutionConfig` from a [`PlanState`], transferring
    /// `step_selection` so partial execution works end-to-end.
    fn from(plan: &PlanState) -> Self {
        Self {
            step_selection: plan.step_selection.clone(),
            ..Self::default()
        }
    }
}

/// Sub-agent result — structured output returned after a sub-agent
/// completes a step.
///
/// Parsed from the sub-agent's notification text (see [`notification`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    /// Index of the completed step (0-based).
    pub step_index: usize,
    /// Final status of the step after execution.
    pub status: ExecutionStepStatus,
    /// Human-readable summary of what was done.
    pub summary: String,
    /// Files that were modified during execution.
    pub changed_files: Vec<String>,
    /// Error message if the step failed.
    pub error_message: Option<String>,
}

impl Default for SubAgentResult {
    fn default() -> Self {
        Self {
            step_index: 0,
            status: ExecutionStepStatus::Pending,
            summary: String::new(),
            changed_files: Vec::new(),
            error_message: None,
        }
    }
}
