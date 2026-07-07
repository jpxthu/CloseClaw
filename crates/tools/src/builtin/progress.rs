//! Built-in tool — Progress.
//!
//! Allows the LLM to update plan execution step progress.
//! All state transitions are validated by [`PlanState`] to enforce
//! the step state machine rules.

use crate::{Tool, ToolCallError, ToolFlags, ToolResult};

use async_trait::async_trait;

use closeclaw_common::{ExecutionStepStatus, PlanState, TransitionError};
use serde_json::Value;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// ProgressTool
// ---------------------------------------------------------------------------

/// Tool that updates plan execution step progress.
///
/// The LLM calls this tool to mark a step as in_progress, completed,
/// failed, or skipped. The tool validates the transition against the
/// step state machine rules and updates the [`PlanState`] accordingly.
///
/// # State machine
///
/// ```text
/// pending → in_progress → completed | failed
///                    ↑         ↛
///                    └ failed ─┘
/// pending → skipped
/// ```
///
/// Rules:
/// - `in_progress`: current status must be `pending` or `failed`
/// - `completed`: current status must be `in_progress`
/// - `failed`: current status must be `in_progress`
/// - `skipped`: current status must be `pending`
/// - Step skipping is forbidden: target step_index must equal
///   `current_step` (or 0 if no step started yet)
pub struct ProgressTool {
    plan_state: Arc<Mutex<PlanState>>,
}

impl ProgressTool {
    /// Create a new `ProgressTool` backed by the given `PlanState`.
    pub fn new(plan_state: Arc<Mutex<PlanState>>) -> Self {
        Self { plan_state }
    }

    /// Validate the transition and apply it to the inner `PlanState`.
    fn apply(
        &self,
        step_index: usize,
        status: ExecutionStepStatus,
        summary: Option<String>,
        error_message: Option<String>,
    ) -> Result<(), ToolCallError> {
        let mut ps = self
            .plan_state
            .lock()
            .map_err(|e| ToolCallError::ExecutionFailed(e.to_string()))?;

        ps.apply_transition(step_index, status)
            .map_err(Self::transition_to_tool_error)?;

        // Attach summary / error_message if provided
        if let Some(s) = summary {
            ps.execution_steps[step_index].summary = s;
        }
        if let Some(e) = error_message {
            ps.execution_steps[step_index].error_message = Some(e);
        }

        Ok(())
    }

    /// Map a [`TransitionError`] into the appropriate [`ToolCallError`].
    fn transition_to_tool_error(e: TransitionError) -> ToolCallError {
        match e {
            TransitionError::OutOfBounds { index, len } => {
                ToolCallError::InvalidArgs(format!("step_index {index} out of range (len {len})"))
            }
            TransitionError::SkippedStep { expected, got } => ToolCallError::InvalidArgs(format!(
                "cannot skip step: expected {expected}, got {got}"
            )),
            TransitionError::InvalidTransition { from, to } => {
                ToolCallError::InvalidArgs(format!("invalid transition: {from:?} -> {to:?}"))
            }
        }
    }

    /// Parse and validate the input arguments.
    fn parse_args(args: &Value) -> Result<ParsedArgs, ToolCallError> {
        let step_index = args
            .get("step_index")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ToolCallError::InvalidArgs(
                    "missing or invalid `step_index` (expected integer)".to_string(),
                )
            })? as usize;

        let status_str = args.get("status").and_then(Value::as_str).ok_or_else(|| {
            ToolCallError::InvalidArgs("missing or invalid `status` (expected string)".to_string())
        })?;

        let status = Self::parse_status(status_str)?;
        let summary = args
            .get("summary")
            .and_then(Value::as_str)
            .map(String::from);
        let error_message = args
            .get("error_message")
            .and_then(Value::as_str)
            .map(String::from);

        Ok(ParsedArgs {
            step_index,
            status,
            summary,
            error_message,
        })
    }

    /// Convert a status string to [`ExecutionStepStatus`].
    fn parse_status(s: &str) -> Result<ExecutionStepStatus, ToolCallError> {
        match s {
            "in_progress" => Ok(ExecutionStepStatus::InProgress),
            "completed" => Ok(ExecutionStepStatus::Completed),
            "failed" => Ok(ExecutionStepStatus::Failed),
            "skipped" => Ok(ExecutionStepStatus::Skipped),
            other => Err(ToolCallError::InvalidArgs(format!(
                "unknown status `{other}`; expected one of: in_progress, completed, failed, skipped"
            ))),
        }
    }
}

/// Parsed and validated input arguments for ProgressTool.
struct ParsedArgs {
    step_index: usize,
    status: ExecutionStepStatus,
    summary: Option<String>,
    error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Tool for ProgressTool {
    fn name(&self) -> &str {
        "Progress"
    }

    fn group(&self) -> &str {
        "plan"
    }

    fn summary(&self) -> String {
        "Update plan execution step progress".to_string()
    }

    fn detail(&self) -> String {
        "Update the progress of a plan execution step. \
         Call this tool to mark a step as in_progress, completed, failed, or skipped.\n\n\
         **State machine**:\n\
         - `pending → in_progress → completed | failed`\n\
         - `pending → skipped`\n\
         - `failed → in_progress` (retry allowed)\n\n\
         **Rules**:\n\
         - `in_progress`: current status must be `pending` or `failed`\n\
         - `completed`: current status must be `in_progress`\n\
         - `failed`: current status must be `in_progress`\n\
         - `skipped`: current status must be `pending`\n\
         - Step skipping is forbidden: `step_index` must equal \
         the current step or 0 (if no step started yet)\n\
         - `completed` steps cannot be reverted"
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "step_index": {
                    "type": "integer",
                    "description": "Index of the step to update (0-based)"
                },
                "status": {
                    "type": "string",
                    "enum": ["in_progress", "completed", "failed", "skipped"],
                    "description": "New status for the step"
                },
                "summary": {
                    "type": "string",
                    "description": "Optional summary of the step result"
                },
                "error_message": {
                    "type": "string",
                    "description": "Optional error message (for failed status)"
                }
            },
            "required": ["step_index", "status"]
        })
    }

    async fn call(
        &self,
        args: Value,
        _ctx: &crate::ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let parsed = Self::parse_args(&args)?;
        self.apply(
            parsed.step_index,
            parsed.status,
            parsed.summary,
            parsed.error_message,
        )?;

        Ok(ToolResult {
            data: serde_json::json!({
                "success": true,
                "step_index": parsed.step_index,
                "status": parsed.status,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: false,
            is_destructive: false,
            is_expensive: false,
            is_deferred_by_default: false,
        }
    }
}
