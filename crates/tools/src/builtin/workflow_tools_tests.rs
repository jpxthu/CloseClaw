//! Unit tests for workflow tools: metadata, input_schema, flags,
//! and call() happy / error paths.

use crate::{Tool, ToolCallError, ToolResult};
use serde_json::json;

use super::test_helpers::test_ctx;

fn assert_tool_metadata(tool: &dyn Tool, expected_name: &str, expected_group: &str) {
    assert_eq!(tool.name(), expected_name);
    assert_eq!(tool.group(), expected_group);
    let summary = tool.summary();
    assert!(!summary.is_empty(), "summary must not be empty");
    assert!(
        summary.len() <= 50,
        "summary '{}' exceeds 50 chars (len={})",
        summary,
        summary.len()
    );
}

fn assert_flags(tool: &dyn Tool) {
    let flags = tool.flags();
    assert!(
        !flags.is_deferred_by_default,
        "is_deferred_by_default must be false for workflow tools"
    );
    assert!(
        !flags.is_concurrency_safe,
        "is_concurrency_safe must be false"
    );
}

// ===========================================================================
// WorkflowStartTool
// ===========================================================================

#[tokio::test]
async fn test_start_metadata() {
    let tool = super::WorkflowStartTool;
    assert_tool_metadata(&tool, "workflow_start", "workflow");
}

#[tokio::test]
async fn test_start_summary_matches_plan() {
    let tool = super::WorkflowStartTool;
    assert_eq!(tool.summary(), "Start a workflow by name");
}

#[tokio::test]
async fn test_start_flags() {
    let tool = super::WorkflowStartTool;
    assert_flags(&tool);
}

#[tokio::test]
async fn test_start_input_schema_requires_name() {
    let tool = super::WorkflowStartTool;
    let schema = tool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "name");
}

#[tokio::test]
async fn test_start_call_happy_path() {
    let tool = super::WorkflowStartTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"name": "design-doc-modify"}), &ctx).await;
    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    let ToolResult {
        data,
        new_messages,
        context_modifier,
    } = result.unwrap();
    assert_eq!(data["action"], "workflow_start");
    assert_eq!(data["name"], "design-doc-modify");
    assert!(new_messages.is_empty());
    assert!(context_modifier.is_none());
}

#[tokio::test]
async fn test_start_call_empty_name_returns_error() {
    let tool = super::WorkflowStartTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"name": ""}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_start_call_missing_name_returns_error() {
    let tool = super::WorkflowStartTool;
    let ctx = test_ctx();
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

// ===========================================================================
// WorkflowVerifyTool
// ===========================================================================

#[tokio::test]
async fn test_verify_metadata() {
    let tool = super::WorkflowVerifyTool;
    assert_tool_metadata(&tool, "workflow_verify", "workflow");
}

#[tokio::test]
async fn test_verify_summary_matches_plan() {
    let tool = super::WorkflowVerifyTool;
    assert_eq!(tool.summary(), "Declare current step complete");
}

#[tokio::test]
async fn test_verify_flags() {
    let tool = super::WorkflowVerifyTool;
    assert_flags(&tool);
}

#[tokio::test]
async fn test_verify_input_schema_no_required() {
    let tool = super::WorkflowVerifyTool;
    let schema = tool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
    assert!(
        required.is_empty(),
        "verify tool should have no required params"
    );
}

#[tokio::test]
async fn test_verify_call_happy_path() {
    let tool = super::WorkflowVerifyTool;
    let ctx = test_ctx();
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_ok());
    let ToolResult {
        data,
        new_messages,
        context_modifier,
    } = result.unwrap();
    assert_eq!(data["action"], "workflow_verify");
    assert!(new_messages.is_empty());
    assert!(context_modifier.is_none());
}

#[tokio::test]
async fn test_verify_call_ignores_extra_args() {
    let tool = super::WorkflowVerifyTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"extra": "ignored"}), &ctx).await;
    assert!(result.is_ok());
}

// ===========================================================================
// WorkflowJumpTool
// ===========================================================================

#[tokio::test]
async fn test_jump_metadata() {
    let tool = super::WorkflowJumpTool;
    assert_tool_metadata(&tool, "workflow_jump", "workflow");
}

#[tokio::test]
async fn test_jump_summary_matches_plan() {
    let tool = super::WorkflowJumpTool;
    assert_eq!(tool.summary(), "Answer jump questions to proceed");
}

#[tokio::test]
async fn test_jump_flags() {
    let tool = super::WorkflowJumpTool;
    assert_flags(&tool);
}

#[tokio::test]
async fn test_jump_input_schema_requires_answers() {
    let tool = super::WorkflowJumpTool;
    let schema = tool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "answers");
}

#[tokio::test]
async fn test_jump_call_happy_path() {
    let tool = super::WorkflowJumpTool;
    let ctx = test_ctx();
    let answers = json!({"go_next": true, "path": "fast"});
    let result = tool.call(json!({"answers": answers}), &ctx).await;
    assert!(result.is_ok());
    let ToolResult {
        data,
        new_messages,
        context_modifier,
    } = result.unwrap();
    assert_eq!(data["action"], "workflow_jump");
    assert_eq!(data["answers"]["go_next"], true);
    assert_eq!(data["answers"]["path"], "fast");
    assert!(new_messages.is_empty());
    assert!(context_modifier.is_none());
}

#[tokio::test]
async fn test_jump_call_missing_answers_returns_error() {
    let tool = super::WorkflowJumpTool;
    let ctx = test_ctx();
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_jump_call_answers_not_object_returns_error() {
    let tool = super::WorkflowJumpTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"answers": "not-an-object"}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_jump_call_answers_array_returns_error() {
    let tool = super::WorkflowJumpTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"answers": [1, 2, 3]}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

// ===========================================================================
// WorkflowBlockedTool
// ===========================================================================

#[tokio::test]
async fn test_blocked_metadata() {
    let tool = super::WorkflowBlockedTool;
    assert_tool_metadata(&tool, "workflow_blocked", "workflow");
}

#[tokio::test]
async fn test_blocked_summary_matches_plan() {
    let tool = super::WorkflowBlockedTool;
    assert_eq!(tool.summary(), "Request to block workflow");
}

#[tokio::test]
async fn test_blocked_flags() {
    let tool = super::WorkflowBlockedTool;
    assert_flags(&tool);
}

#[tokio::test]
async fn test_blocked_input_schema_requires_reason() {
    let tool = super::WorkflowBlockedTool;
    let schema = tool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0], "reason");
}

#[tokio::test]
async fn test_blocked_call_happy_path() {
    let tool = super::WorkflowBlockedTool;
    let ctx = test_ctx();
    let result = tool
        .call(json!({"reason": "waiting for owner approval"}), &ctx)
        .await;
    assert!(result.is_ok());
    let ToolResult {
        data,
        new_messages,
        context_modifier,
    } = result.unwrap();
    assert_eq!(data["action"], "workflow_blocked");
    assert_eq!(data["reason"], "waiting for owner approval");
    assert!(new_messages.is_empty());
    assert!(context_modifier.is_none());
}

#[tokio::test]
async fn test_blocked_call_empty_reason_returns_error() {
    let tool = super::WorkflowBlockedTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"reason": ""}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_blocked_call_missing_reason_returns_error() {
    let tool = super::WorkflowBlockedTool;
    let ctx = test_ctx();
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}

#[tokio::test]
async fn test_blocked_call_reason_not_string_returns_error() {
    let tool = super::WorkflowBlockedTool;
    let ctx = test_ctx();
    let result = tool.call(json!({"reason": 123}), &ctx).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
}
