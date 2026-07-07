//! Tests for ProgressTool.

use super::progress::ProgressTool;
use crate::{Tool, ToolCallError};
use closeclaw_common::{ExecutionStepStatus, PlanState};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Helper: create a ProgressTool with an initialized 3-step PlanState.
fn make_tool() -> (ProgressTool, Arc<Mutex<PlanState>>) {
    let mut ps = PlanState::new();
    ps.init_execution_steps(vec!["Step 1".into(), "Step 2".into(), "Step 3".into()]);
    let ps = Arc::new(Mutex::new(ps));
    let tool = ProgressTool::new(Arc::clone(&ps));
    (tool, ps)
}

/// Helper: make a minimal ToolContext for testing.
fn test_ctx() -> crate::ToolContext {
    crate::ToolContext {
        agent_id: "test".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

#[tokio::test]
async fn test_progress_tool_name_group() {
    let (tool, _) = make_tool();
    assert_eq!(tool.name(), "Progress");
    assert_eq!(tool.group(), "plan");
}

#[tokio::test]
async fn test_progress_tool_summary_len() {
    let (tool, _) = make_tool();
    assert!(tool.summary().len() <= 50);
}

#[tokio::test]
async fn test_progress_tool_flags() {
    let (tool, _) = make_tool();
    let flags = tool.flags();
    assert!(!flags.is_concurrency_safe);
    assert!(!flags.is_read_only);
    assert!(!flags.is_destructive);
    assert!(!flags.is_deferred_by_default);
}

#[tokio::test]
async fn test_progress_tool_valid_transition_in_progress() {
    let (tool, ps) = make_tool();
    let result = tool
        .call(
            json!({"step_index": 0, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *ps.lock().unwrap().get_step_status(0).unwrap(),
        ExecutionStepStatus::InProgress
    );
}

#[tokio::test]
async fn test_progress_tool_valid_transition_completed() {
    let (tool, ps) = make_tool();
    // pending → in_progress
    tool.call(
        json!({"step_index": 0, "status": "in_progress"}),
        &test_ctx(),
    )
    .await
    .unwrap();
    // in_progress → completed
    let result = tool
        .call(json!({"step_index": 0, "status": "completed"}), &test_ctx())
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *ps.lock().unwrap().get_step_status(0).unwrap(),
        ExecutionStepStatus::Completed
    );
}

#[tokio::test]
async fn test_progress_tool_valid_transition_failed() {
    let (tool, ps) = make_tool();
    tool.call(
        json!({"step_index": 0, "status": "in_progress"}),
        &test_ctx(),
    )
    .await
    .unwrap();
    let result = tool
        .call(json!({"step_index": 0, "status": "failed"}), &test_ctx())
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *ps.lock().unwrap().get_step_status(0).unwrap(),
        ExecutionStepStatus::Failed
    );
}

#[tokio::test]
async fn test_progress_tool_retry_after_failure() {
    let (tool, ps) = make_tool();
    // pending → in_progress
    tool.call(
        json!({"step_index": 0, "status": "in_progress"}),
        &test_ctx(),
    )
    .await
    .unwrap();
    // in_progress → failed
    tool.call(json!({"step_index": 0, "status": "failed"}), &test_ctx())
        .await
        .unwrap();
    // failed → in_progress (retry)
    let result = tool
        .call(
            json!({"step_index": 0, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *ps.lock().unwrap().get_step_status(0).unwrap(),
        ExecutionStepStatus::InProgress
    );
}

#[tokio::test]
async fn test_progress_tool_completed_cannot_go_back() {
    let (tool, _ps) = make_tool();
    tool.call(
        json!({"step_index": 0, "status": "in_progress"}),
        &test_ctx(),
    )
    .await
    .unwrap();
    tool.call(json!({"step_index": 0, "status": "completed"}), &test_ctx())
        .await
        .unwrap();
    // completed → in_progress should fail
    // After step 0 is completed, current_step advances to 1,
    // so the skip-step check rejects step_index=0 first.
    let result = tool
        .call(
            json!({"step_index": 0, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(
        result.is_err(),
        "expected error for completed -> in_progress"
    );
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("cannot skip step") || msg.contains("invalid transition"),
                "unexpected error message: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_progress_tool_skip_step_rejected() {
    let (tool, _) = make_tool();
    // Step 0 is current, trying to start step 1 should fail
    let result = tool
        .call(
            json!({"step_index": 1, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("cannot skip step"));
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_progress_tool_out_of_bounds() {
    let (tool, _ps) = make_tool();
    let result = tool
        .call(
            json!({"step_index": 10, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("out of range"));
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_progress_tool_missing_params() {
    let (tool, _) = make_tool();
    // Missing step_index
    let result = tool
        .call(json!({"status": "in_progress"}), &test_ctx())
        .await;
    assert!(result.is_err());
    // Missing status
    let result = tool.call(json!({"step_index": 0}), &test_ctx()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_progress_tool_unknown_status() {
    let (tool, _) = make_tool();
    let result = tool
        .call(json!({"step_index": 0, "status": "bogus"}), &test_ctx())
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("unknown status"));
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn test_progress_tool_skip_from_pending() {
    let (tool, ps) = make_tool();
    let result = tool
        .call(json!({"step_index": 0, "status": "skipped"}), &test_ctx())
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *ps.lock().unwrap().get_step_status(0).unwrap(),
        ExecutionStepStatus::Skipped
    );
}

#[tokio::test]
async fn test_progress_tool_full_flow() {
    let (tool, ps) = make_tool();
    let ctx = test_ctx();

    // Step 0: in_progress → completed
    tool.call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(
        json!({"step_index": 0, "status": "completed", "summary": "done"}),
        &ctx,
    )
    .await
    .unwrap();

    // Step 1: in_progress → failed → in_progress (retry) → completed
    tool.call(json!({"step_index": 1, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(
        json!({"step_index": 1, "status": "failed", "error_message": "oops"}),
        &ctx,
    )
    .await
    .unwrap();
    tool.call(json!({"step_index": 1, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(json!({"step_index": 1, "status": "completed"}), &ctx)
        .await
        .unwrap();

    // Step 2: in_progress → skipped
    tool.call(json!({"step_index": 2, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(json!({"step_index": 2, "status": "completed"}), &ctx)
        .await
        .unwrap();

    let ps = ps.lock().unwrap();
    assert_eq!(
        *ps.get_step_status(0).unwrap(),
        ExecutionStepStatus::Completed
    );
    assert_eq!(
        *ps.get_step_status(1).unwrap(),
        ExecutionStepStatus::Completed
    );
    assert_eq!(
        *ps.get_step_status(2).unwrap(),
        ExecutionStepStatus::Completed
    );
}

#[tokio::test]
async fn test_progress_tool_summary_and_error_message_applied() {
    let (tool, ps) = make_tool();
    let ctx = test_ctx();

    tool.call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(
        json!({"step_index": 0, "status": "failed", "error_message": "runtime error", "summary": "partial work"}),
        &ctx,
    )
    .await
    .unwrap();

    let ps = ps.lock().unwrap();
    let step = &ps.execution_steps[0];
    assert_eq!(step.error_message.as_deref(), Some("runtime error"));
    assert_eq!(step.summary, "partial work");
}

#[tokio::test]
async fn test_progress_tool_input_schema() {
    let (tool, _) = make_tool();
    let schema = tool.input_schema();
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("step_index")));
    assert!(required.contains(&json!("status")));
    let props = schema.pointer("/properties").unwrap().as_object().unwrap();
    assert!(props.contains_key("step_index"));
    assert!(props.contains_key("status"));
    assert!(props.contains_key("summary"));
    assert!(props.contains_key("error_message"));
}

#[tokio::test]
async fn test_progress_tool_detail_contains_rules() {
    let (tool, _) = make_tool();
    let detail = tool.detail();
    assert!(detail.contains("State machine"));
    assert!(detail.contains("in_progress"));
    assert!(detail.contains("completed"));
    assert!(detail.contains("failed"));
}
