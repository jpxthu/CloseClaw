//! Workflow run state types.

use serde::{Deserialize, Serialize};

/// Execution phases of a workflow step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Agent is executing step content.
    Executing,
    /// Engine has injected verify checklist, waiting for Agent response.
    Verifying,
    /// Engine has injected jump questions, waiting for Agent answers.
    Jumping,
    /// Blocked waiting for owner intervention.
    Blocked,
    /// Workflow has completed.
    Complete,
}

/// Entry in the step history recording completed steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepHistoryEntry {
    /// The step index that was completed.
    pub step_id: usize,
    /// The step name at time of completion.
    pub step_name: String,
    /// ISO 8601 timestamp when the step was completed.
    pub completed_at: String,
}

/// Runtime state of a workflow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// ID of the workflow being executed.
    pub workflow_id: String,
    /// Version of the workflow definition.
    pub definition_version: String,
    /// Index of the current step (0-based).
    pub current_step: usize,
    /// Current execution phase.
    pub phase: Phase,
    /// History of completed steps.
    pub step_history: Vec<StepHistoryEntry>,
    /// Cross-step shared data.
    #[serde(default)]
    pub step_data: serde_yaml::Value,
    /// Number of verify attempts since last reset.
    #[serde(default)]
    pub pending_verify: usize,
}
