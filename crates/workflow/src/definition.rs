//! Workflow definition types and YAML frontmatter parsing.

use serde::{Deserialize, Serialize};

/// A complete workflow definition parsed from YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier for this workflow.
    pub id: String,
    /// Human-readable workflow name.
    pub name: String,
    /// Description of what this workflow does.
    pub description: String,
    /// Whether Agent can call workflow_blocked (default: false).
    #[serde(default)]
    pub allow_blocked: bool,
    /// Verify retry limit (default: 3).
    #[serde(default = "default_verify_retry_limit")]
    pub verify_retry_limit: usize,
    /// Cross-step shared data field declarations: { field_name: type }.
    #[serde(default)]
    pub step_data_schema: serde_yaml::Value,
    /// Ordered list of steps in this workflow.
    pub steps: Vec<Step>,
}

fn default_verify_retry_limit() -> usize {
    3
}

/// A single step within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step index (0-based).
    pub id: usize,
    /// Human-readable step name.
    pub name: String,
    /// Override workflow-level allow_blocked for this step (optional).
    #[serde(default)]
    pub allow_blocked: Option<bool>,
    /// Step goal description (plain text).
    pub goal: String,
    /// Verification checklist items.
    pub verify: Vec<String>,
    /// Jump questions to ask before transitioning.
    #[serde(default)]
    pub jump: Vec<JumpQuestion>,
    /// Transition rules evaluated after jump answers.
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

/// A question asked during the jumping phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpQuestion {
    /// Unique key used as parameter key in workflow_jump.
    pub id: String,
    /// Question description shown to Agent.
    pub prompt: String,
    /// Answer type: "boolean" or "enum".
    #[serde(rename = "type")]
    pub question_type: String,
    /// Option values for enum type (ignored for boolean).
    #[serde(default)]
    pub options: Vec<String>,
    /// Labels displayed in ABCD order (optional, used for rendering).
    #[serde(default)]
    pub option_labels: Vec<String>,
}

/// A transition rule that maps jump answers to an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    /// Match conditions: { jump_id: expected_value, ... }.
    /// Omit for default (fallback) transition.
    #[serde(default)]
    pub when: Option<serde_yaml::Value>,
    /// Action to take: "goto", "reexecute", or "complete".
    pub action: String,
    /// Target step index (required for goto/reexecute).
    #[serde(default)]
    pub target_step: Option<usize>,
}

/// Actions that can result from a jump evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JumpAction {
    /// Move to target step, clear step_data.
    Goto(usize),
    /// Re-enter target step, keep step_data.
    Reexecute(usize),
    /// Workflow complete.
    Complete,
}
