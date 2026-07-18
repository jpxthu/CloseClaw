//! Built-in PlanApproval tool.
//!
//! Provides the Plan Mode approval gate — the exclusive exit from Plan Mode
//! to Auto Mode. When the agent calls this tool with a plan summary, the
//! framework presents a user confirmation dialog. Approval transitions the
//! session from Plan Mode to Auto Mode; rejection keeps the plan in draft
//! status.

use crate::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};

use async_trait::async_trait;
use serde_json::Value;

// ---------------------------------------------------------------------------
// PlanApprovalTool
// ---------------------------------------------------------------------------

/// Plan Mode approval gate tool.
///
/// The agent calls this tool after completing a plan in Plan Mode.
/// The tool returns an `approval_pending` result, prompting the framework
/// to display a user confirmation dialog.
///
/// - **Approval** → the session transitions from Plan Mode to Auto Mode.
/// - **Rejection** → the plan remains in draft; the agent continues editing.
pub struct PlanApprovalTool;

impl PlanApprovalTool {
    /// Creates a new `PlanApprovalTool`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PlanApprovalTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PlanApprovalTool {
    fn name(&self) -> &str {
        "plan_approval"
    }

    fn group(&self) -> &str {
        "plan"
    }

    fn summary(&self) -> String {
        "Submit plan for approval to exit Plan Mode".to_string()
    }

    fn detail(&self) -> String {
        "Submit the current plan for owner approval. This is the exclusive \
         exit from Plan Mode to Auto Mode. The owner will review the plan \
         summary and approve or reject it. \
         \n\nApproval transitions the session to Auto Mode for execution. \
         Rejection keeps the plan in Plan Mode for further editing."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan_summary": {
                    "type": "string",
                    "description": "A concise summary of the plan to be approved"
                },
                "plan_file_path": {
                    "type": "string",
                    "description": "Path to the plan file (optional, used for status update on approval)"
                }
            },
            "required": ["plan_summary"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: false,
            is_destructive: false,
            is_expensive: false,
            is_deferred_by_default: false,
        }
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        let plan_summary = args
            .get("plan_summary")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ToolCallError::InvalidArgs("missing required parameter: plan_summary".to_string())
            })?;

        if plan_summary.trim().is_empty() {
            return Err(ToolCallError::InvalidArgs(
                "plan_summary must not be empty".to_string(),
            ));
        }

        let request_id = uuid::Uuid::new_v4().to_string();
        let plan_file_path = args
            .get("plan_file_path")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        Ok(ToolResult {
            data: super::approval_utils::build_approval_pending_with_plan(
                request_id,
                plan_file_path,
            ),
            new_messages: Vec::new(),
            context_modifier: None,
        })
    }
}
