//! Workflow tools — Tool trait implementations for workflow operations.
//!
//! Provides four tools (`workflow_start`, `workflow_verify`,
//! `workflow_jump`, `workflow_blocked`) that allow an Agent to
//! communicate workflow intent to the Engine via structured tool calls.
//!
//! These tools return structured `ToolResult` payloads describing the
//! intended action. Actual Engine interaction is handled by the session
//! integration layer (split seq 2), which will route the action to the
//! appropriate `WorkflowEngine` method.

use crate::{Tool, ToolCallError, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Shared ToolFlags for all workflow tools.
///
/// Workflow tools are system-level tools that must be immediately
/// visible to the agent, so `is_deferred_by_default` is `false`.
fn workflow_flags() -> ToolFlags {
    ToolFlags {
        is_concurrency_safe: false,
        is_read_only: false,
        is_destructive: false,
        is_expensive: false,
        is_deferred_by_default: false,
    }
}

// ---------------------------------------------------------------------------
// WorkflowStartTool
// ---------------------------------------------------------------------------

/// Start a workflow by name.
///
/// Returns a structured result describing the start action. The session
/// layer will eventually forward this to `WorkflowEngine::start()`.
pub struct WorkflowStartTool;

#[async_trait]
impl Tool for WorkflowStartTool {
    fn name(&self) -> &str {
        "workflow_start"
    }

    fn group(&self) -> &str {
        "workflow"
    }

    fn summary(&self) -> String {
        "Start a workflow by name".to_string()
    }

    fn detail(&self) -> String {
        "Start a workflow by its name. The engine will load the workflow \
         definition and create a new run."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Workflow name"
                }
            },
            "required": ["name"]
        })
    }

    fn flags(&self) -> ToolFlags {
        workflow_flags()
    }

    async fn call(
        &self,
        args: Value,
        _ctx: &crate::ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let name = args.get("name").and_then(Value::as_str).ok_or_else(|| {
            ToolCallError::InvalidArgs("missing required parameter: name".to_string())
        })?;
        if name.is_empty() {
            return Err(ToolCallError::InvalidArgs(
                "name must not be empty".to_string(),
            ));
        }

        Ok(ToolResult {
            data: serde_json::json!({
                "action": "workflow_start",
                "name": name,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

// ---------------------------------------------------------------------------
// WorkflowVerifyTool
// ---------------------------------------------------------------------------

/// Declare the current workflow step complete.
///
/// Returns a structured result describing the verify action.
pub struct WorkflowVerifyTool;

#[async_trait]
impl Tool for WorkflowVerifyTool {
    fn name(&self) -> &str {
        "workflow_verify"
    }

    fn group(&self) -> &str {
        "workflow"
    }

    fn summary(&self) -> String {
        "Declare current step complete".to_string()
    }

    fn detail(&self) -> String {
        "Declare that the current workflow step is complete. The engine \
         will evaluate transitions to determine the next step."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        workflow_flags()
    }

    async fn call(
        &self,
        _args: Value,
        _ctx: &crate::ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        Ok(ToolResult {
            data: serde_json::json!({
                "action": "workflow_verify",
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

// ---------------------------------------------------------------------------
// WorkflowJumpTool
// ---------------------------------------------------------------------------

/// Answer jump questions to proceed in a workflow.
///
/// Returns a structured result describing the jump action with answers.
pub struct WorkflowJumpTool;

#[async_trait]
impl Tool for WorkflowJumpTool {
    fn name(&self) -> &str {
        "workflow_jump"
    }

    fn group(&self) -> &str {
        "workflow"
    }

    fn summary(&self) -> String {
        "Answer jump questions to proceed".to_string()
    }

    fn detail(&self) -> String {
        "Submit answers to jump questions. The engine will evaluate the \
         answers to determine the next step in the workflow."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "answers": {
                    "type": "object",
                    "description": "Jump question answers"
                }
            },
            "required": ["answers"]
        })
    }

    fn flags(&self) -> ToolFlags {
        workflow_flags()
    }

    async fn call(
        &self,
        args: Value,
        _ctx: &crate::ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let answers = args.get("answers").ok_or_else(|| {
            ToolCallError::InvalidArgs("missing required parameter: answers".to_string())
        })?;
        if !answers.is_object() {
            return Err(ToolCallError::InvalidArgs(
                "answers must be an object".to_string(),
            ));
        }

        Ok(ToolResult {
            data: serde_json::json!({
                "action": "workflow_jump",
                "answers": answers,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}

// ---------------------------------------------------------------------------
// WorkflowBlockedTool
// ---------------------------------------------------------------------------

/// Request to block the workflow.
///
/// Returns a structured result describing the blocked action with a reason.
pub struct WorkflowBlockedTool;

#[async_trait]
impl Tool for WorkflowBlockedTool {
    fn name(&self) -> &str {
        "workflow_blocked"
    }

    fn group(&self) -> &str {
        "workflow"
    }

    fn summary(&self) -> String {
        "Request to block workflow".to_string()
    }

    fn detail(&self) -> String {
        "Request to block the workflow with a reason. The workflow will \
         be paused until the block is resolved."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Block reason"
                }
            },
            "required": ["reason"]
        })
    }

    fn flags(&self) -> ToolFlags {
        workflow_flags()
    }

    async fn call(
        &self,
        args: Value,
        _ctx: &crate::ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let reason = args.get("reason").and_then(Value::as_str).ok_or_else(|| {
            ToolCallError::InvalidArgs("missing required parameter: reason".to_string())
        })?;
        if reason.is_empty() {
            return Err(ToolCallError::InvalidArgs(
                "reason must not be empty".to_string(),
            ));
        }

        Ok(ToolResult {
            data: serde_json::json!({
                "action": "workflow_blocked",
                "reason": reason,
            }),
            new_messages: vec![],
            context_modifier: None,
        })
    }
}
