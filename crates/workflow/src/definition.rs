//! Workflow definition types and YAML frontmatter parsing.

use serde::{Deserialize, Serialize};

use crate::error::WorkflowError;

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

impl Workflow {
    /// Parse a YAML string into a Workflow definition.
    ///
    /// Accepts either:
    /// - Raw YAML content (the frontmatter body between `---` delimiters), or
    /// - A full SKILL.md document with `---` delimiters stripped.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::ParseError`] if the YAML is malformed or
    /// contains invalid field types.
    pub fn parse_frontmatter(yaml: &str) -> Result<Self, WorkflowError> {
        // Strip `---` delimiters if present (handles full SKILL.md input)
        let content = yaml
            .strip_prefix("---\n")
            .or_else(|| yaml.strip_prefix("---\r\n"))
            .or_else(|| yaml.strip_prefix("---\r"))
            .unwrap_or(yaml);

        // Strip trailing `---` delimiter (with optional surrounding newlines)
        let content = content.trim_end_matches(['\n', '\r']);
        let content = content.strip_suffix("---").unwrap_or(content);
        let content = content.trim_end();

        let workflow: Workflow =
            serde_yaml::from_str(content).map_err(WorkflowError::from_yaml_error)?;

        Ok(workflow)
    }

    /// Parse a complete SKILL.md file containing YAML frontmatter delimited
    /// by `---` markers.
    ///
    /// Unlike [`parse_frontmatter`], this function extracts the YAML block
    /// between the first pair of `---` delimiters before parsing.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidDefinition`] if no frontmatter
    /// delimiters are found, or [`WorkflowError::ParseError`] if the YAML
    /// content is malformed.
    pub fn parse_skill_md(content: &str) -> Result<Self, WorkflowError> {
        let frontmatter = extract_frontmatter(content)?;
        Self::parse_frontmatter(frontmatter)
    }
}

/// Extract the YAML frontmatter block from a SKILL.md document.
///
/// Looks for lines that are exactly `---` (with optional surrounding
/// whitespace stripped). Returns the content between the first pair.
fn extract_frontmatter(content: &str) -> Result<&str, WorkflowError> {
    let mut lines = content.lines();
    let first = lines
        .next()
        .ok_or_else(|| WorkflowError::invalid_definition("empty SKILL.md file"))?;

    if first.trim() != "---" {
        return Err(WorkflowError::invalid_definition(
            "missing opening `---` delimiter for YAML frontmatter",
        ));
    }

    let mut body = String::new();
    for line in lines {
        if line.trim() == "---" {
            return Ok(Box::leak(body.into_boxed_str()));
        }
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(line);
    }

    Err(WorkflowError::invalid_definition(
        "missing closing `---` delimiter for YAML frontmatter",
    ))
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
    #[serde(default)]
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
