//! Create Workflow Skill — help agents write workflow definitions.
//!
//! Provides two actions:
//! - `create`: structured guidance for writing a new workflow SKILL.md
//! - `validate`: parse a workflow definition via `Workflow::parse_skill_md`
//!   and report pass/fail with error details.

use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use closeclaw_workflow::definition::Workflow;
use serde_json::json;

/// Bundled skill that assists agents in creating and validating workflow
/// definition files (`SKILL.md` with YAML frontmatter).
pub struct WorkflowCreatorSkill;

impl Default for WorkflowCreatorSkill {
    fn default() -> Self {
        Self
    }
}

impl WorkflowCreatorSkill {
    /// Construct a new instance.
    pub fn new() -> Self {
        Self
    }

    /// Return JSON describing available actions when no action is given.
    fn capabilities_response() -> String {
        json!({
            "skill": "create_workflow",
            "description": "Helps agents create and validate workflow definitions",
            "supported_actions": ["create", "validate"],
            "usage": {
                "create": {"name": "<workflow_name>", "description": "<one-line description>"},
                "validate": {"path": "<path_to_skill_md>"}
            }
        })
        .to_string()
    }

    /// Build the structured create guidance.
    fn build_create_guidance(name: &str, description: &str) -> String {
        json!({
            "skill": "create_workflow",
            "action": "create",
            "target": {
                "file": format!("workflows/{name}/SKILL.md"),
                "description": description
            },
            "frontmatter_template": {
                "id": name,
                "name": name,
                "description": description,
                "version": "0.1",
                "allow_blocked": false,
                "verify_retry_limit": 3,
                "step_data_schema": {},
                "steps": [
                    {
                        "id": 1,
                        "name": "Step Name",
                        "instructions": "What the Agent should do in this step",
                        "transitions": [
                            {
                                "condition": "The condition text the user selects",
                                "next": 2
                            },
                            {
                                "condition": "default",
                                "next": 2,
                                "is_default": true
                            }
                        ]
                    }
                ]
            },
            "frontmatter_fields": {
                "id": "Unique workflow identifier (string)",
                "name": "Human-readable workflow name",
                "description": "One-line description of the workflow",
                "version": "Definition version string (default: \"0.1\")",
                "allow_blocked": "Whether Agent can call workflow_blocked (default: false)",
                "verify_retry_limit": "Max verify retries per step (default: 3)",
                "step_data_schema": "Cross-step shared data fields as { field: type }",
                "steps": "Ordered list of workflow steps"
            },
            "step_fields": {
                "id": "Unique integer step identifier (1-based, sequential)",
                "name": "Human-readable step name",
                "instructions": "Text instructions for the Agent",
                "transitions": "Ordered list of possible transitions to next steps"
            },
            "transition_rules": [
                "Each transition has a condition string and a next step id",
                "Conditions must be unique within a step (no duplicate conditions)",
                "A default transition with condition=\"default\" and is_default=true is required",
                "The default transition must be the last entry in the transitions list",
                "next must reference a valid step id that exists in the workflow"
            ],
            "writing_principles": [
                "Write instructions for both human readers and Agents",
                "Use clear, unambiguous language",
                "Keep steps focused on a single responsibility",
                "Transitions cover all meaningful outcomes"
            ],
            "instructions": "Use `write` to create the file. Frontmatter must be delimited by --- markers. Changes take effect on next agent startup."
        })
        .to_string()
    }

    /// Read a file and validate it as a workflow definition.
    ///
    /// Returns structured JSON with the validation result.
    async fn validate_file(path: &str) -> Result<String, SkillError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SkillError::ExecutionFailed(format!("failed to read '{path}': {e}")))?;

        match Workflow::parse_skill_md(&content) {
            Ok(_) => Ok(json!({
                "skill": "create_workflow",
                "action": "validate",
                "path": path,
                "valid": true,
                "message": "Workflow definition is valid"
            })
            .to_string()),
            Err(ref e) => Ok(json!({
                "skill": "create_workflow",
                "action": "validate",
                "path": path,
                "valid": false,
                "error": e.to_string()
            })
            .to_string()),
        }
    }
}

#[async_trait]
impl Skill for WorkflowCreatorSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "create_workflow".to_string(),
            description: "Helps agents create and validate workflow definitions".to_string(),
            when_to_use: "Use when the agent needs to create or validate a \
                workflow definition (SKILL.md with workflow frontmatter)"
                .to_string(),
            context: crate::disk::types::SkillContext::default(),
            effort: SkillEffort::Small,
            paths: vec![],
            user_invocable: true,
        }
    }

    fn body(&self) -> &str {
        r#"# Create Workflow

Use this skill to create or validate workflow definition files.

## Writing Principles

- **Write for humans and Agents**: instructions should be clear enough for
  a human reviewer yet precise enough for an Agent to execute.
- **One responsibility per step**: each step should accomplish a single
  focused task.
- **Cover all outcomes**: every meaningful branch in a step should have a
  corresponding transition.
- **Use unambiguous language**: avoid vague terms; prefer concrete actions.

## Workflow File Structure

A workflow definition lives at `workflows/<name>/SKILL.md` and consists of:

1. **YAML frontmatter** (between `---` delimiters) containing:
   - `id`, `name`, `description`, `version`
   - `allow_blocked`, `verify_retry_limit`, `step_data_schema`
   - `steps` — ordered list of workflow steps
2. **Markdown body** — supplementary documentation for human readers

## Frontmatter Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| id | string | required | Unique workflow identifier |
| name | string | required | Human-readable name |
| description | string | required | One-line description |
| version | string | "0.1" | Definition version |
| allow_blocked | bool | false | Allow Agent to call workflow_blocked |
| verify_retry_limit | int | 3 | Max verify retries per step |
| step_data_schema | object | {} | Cross-step shared data fields |
| steps | list | required | Ordered workflow steps |

## Step Fields

| Field | Type | Description |
|-------|------|-------------|
| id | int | Unique 1-based sequential identifier |
| name | string | Human-readable step name |
| instructions | string | Text instructions for the Agent |
| transitions | list | Possible transitions to next steps |

## Transition Rules

- Each transition has a `condition` (string) and `next` (step id).
- Conditions must be unique within a step.
- A `default` transition (condition="default", is_default=true) is **required**.
- The default transition must be the **last** entry in the transitions list.
- `next` must reference a step id that exists in the workflow.

## Changes

Changes take effect on the next agent startup."#
    }

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_response()),
        };

        let action = args.get("action").and_then(|v| v.as_str());

        match action {
            None => Ok(Self::capabilities_response()),
            Some("create") => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'name' for create action".into())
                })?;
                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("New workflow");
                Ok(Self::build_create_guidance(name, description))
            }
            Some("validate") => {
                let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for validate action".into())
                })?;
                Self::validate_file(path).await
            }
            Some(other) => Err(SkillError::InvalidArgs(format!(
                "unknown action '{other}', supported: create, validate"
            ))),
        }
    }
}
