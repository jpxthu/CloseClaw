//! Tests for PlanApprovalTool.

use crate::{Tool, ToolCallError, ToolContext};
use serde_json::json;

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
    }
}

#[test]
fn test_plan_approval_name() {
    let tool = super::PlanApprovalTool::new();
    assert_eq!(tool.name(), "plan_approval");
}

#[test]
fn test_plan_approval_group() {
    let tool = super::PlanApprovalTool::new();
    assert_eq!(tool.group(), "plan");
}

#[test]
fn test_plan_approval_summary_length() {
    let tool = super::PlanApprovalTool::new();
    assert!(tool.summary().len() <= 50);
}

#[test]
fn test_plan_approval_summary_content() {
    let tool = super::PlanApprovalTool::new();
    assert!(tool.summary().contains("plan"));
}

#[test]
fn test_plan_approval_flags() {
    let tool = super::PlanApprovalTool::new();
    let flags = tool.flags();
    assert!(!flags.is_read_only);
    assert!(!flags.is_destructive);
    assert!(!flags.is_deferred_by_default);
}

#[test]
fn test_plan_approval_detail_mentions_plan_mode() {
    let tool = super::PlanApprovalTool::new();
    let detail = tool.detail();
    assert!(detail.contains("Plan Mode"));
    assert!(detail.contains("Auto Mode"));
}

#[test]
fn test_plan_approval_input_schema_requires_plan_summary() {
    let tool = super::PlanApprovalTool::new();
    let schema = tool.input_schema();
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("plan_summary")));
}

#[test]
fn test_plan_approval_input_schema_has_plan_summary_property() {
    let tool = super::PlanApprovalTool::new();
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    assert!(props.get("plan_summary").is_some());
}

#[tokio::test]
async fn test_plan_approval_call_success() {
    let tool = super::PlanApprovalTool::new();
    let result = tool
        .call(
            json!({"plan_summary": "Implement user auth flow"}),
            &make_ctx(),
        )
        .await;
    assert!(result.is_ok(), "should succeed, got: {:?}", result.err());
    let output = result.unwrap();
    assert_eq!(output.data["status"], "approval_pending");
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id"
    );
}

#[tokio::test]
async fn test_plan_approval_call_empty_summary() {
    let tool = super::PlanApprovalTool::new();
    let result = tool.call(json!({"plan_summary": ""}), &make_ctx()).await;
    assert!(result.is_err(), "empty summary should fail");
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("must not be empty"));
        }
        other => panic!("expected InvalidArgs, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_plan_approval_call_whitespace_only_summary() {
    let tool = super::PlanApprovalTool::new();
    let result = tool.call(json!({"plan_summary": "   "}), &make_ctx()).await;
    assert!(result.is_err(), "whitespace-only summary should fail");
}

#[tokio::test]
async fn test_plan_approval_call_missing_summary() {
    let tool = super::PlanApprovalTool::new();
    let result = tool.call(json!({}), &make_ctx()).await;
    assert!(result.is_err(), "missing summary should fail");
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("missing required parameter"));
        }
        other => panic!("expected InvalidArgs, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_plan_approval_returns_unique_request_ids() {
    let tool = super::PlanApprovalTool::new();
    let r1 = tool
        .call(json!({"plan_summary": "plan A"}), &make_ctx())
        .await
        .unwrap();
    let r2 = tool
        .call(json!({"plan_summary": "plan B"}), &make_ctx())
        .await
        .unwrap();
    assert_ne!(
        r1.data["request_id"], r2.data["request_id"],
        "request IDs should be unique"
    );
}
