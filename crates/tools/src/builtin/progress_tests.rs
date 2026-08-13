//! Tests for ProgressTool.

use super::progress::ProgressTool;
use crate::{Tool, ToolCallError};
use closeclaw_common::PlanState;
use closeclaw_execution::{
    get_step_status, init_execution_steps, DefaultPlanStateWriter, ExecutionState,
    ExecutionStepStatus, PlanStateWriter,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

use super::test_helpers::test_ctx;

/// Helper: create a ProgressTool with an initialized 3-step ExecutionState.
fn make_tool() -> (ProgressTool, Arc<Mutex<ExecutionState>>) {
    let mut es = ExecutionState::new();
    init_execution_steps(
        &mut es,
        vec!["Step 1".into(), "Step 2".into(), "Step 3".into()],
    );
    let es = Arc::new(Mutex::new(es));
    let ps = Arc::new(Mutex::new(PlanState::new()));
    let tool = ProgressTool::new(Arc::clone(&es), ps);
    (tool, es)
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
    let (tool, es) = make_tool();
    let result = tool
        .call(
            json!({"step_index": 0, "status": "in_progress"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::InProgress
    );
}

#[tokio::test]
async fn test_progress_tool_valid_transition_completed() {
    let (tool, es) = make_tool();
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
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::Completed
    );
}

#[tokio::test]
async fn test_progress_tool_valid_transition_failed() {
    let (tool, es) = make_tool();
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
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::Failed
    );
}

#[tokio::test]
async fn test_progress_tool_retry_after_failure() {
    let (tool, es) = make_tool();
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
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::InProgress
    );
}

#[tokio::test]
async fn test_progress_tool_completed_cannot_go_back() {
    let (tool, _es) = make_tool();
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
    let (tool, _es) = make_tool();
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
    let (tool, es) = make_tool();
    let result = tool
        .call(json!({"step_index": 0, "status": "skipped"}), &test_ctx())
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::Skipped
    );
}

#[tokio::test]
async fn test_progress_tool_full_flow() {
    let (tool, es) = make_tool();
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

    // Step 2: in_progress → completed
    tool.call(json!({"step_index": 2, "status": "in_progress"}), &ctx)
        .await
        .unwrap();
    tool.call(json!({"step_index": 2, "status": "completed"}), &ctx)
        .await
        .unwrap();

    let es = es.lock().unwrap();
    assert_eq!(
        *get_step_status(&es, 0).unwrap(),
        ExecutionStepStatus::Completed
    );
    assert_eq!(
        *get_step_status(&es, 1).unwrap(),
        ExecutionStepStatus::Completed
    );
    assert_eq!(
        *get_step_status(&es, 2).unwrap(),
        ExecutionStepStatus::Completed
    );
}

#[tokio::test]
async fn test_progress_tool_summary_and_error_message_applied() {
    let (tool, es) = make_tool();
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

    let es = es.lock().unwrap();
    let step = &es.execution_steps[0];
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

// ---------------------------------------------------------------------------
// Plan file synchronization tests
// ---------------------------------------------------------------------------

/// Helper: create a ProgressTool with writer and a PlanState whose
/// `plan_file_path` points to the given temp file.
fn make_tool_with_writer(plan_file_path: &str) -> (ProgressTool, Arc<Mutex<ExecutionState>>) {
    let mut es = ExecutionState::new();
    init_execution_steps(&mut es, vec!["Step 1".into(), "Step 2".into()]);
    let es = Arc::new(Mutex::new(es));
    let mut ps = PlanState::new();
    ps.plan_file_path = plan_file_path.to_string();
    let ps = Arc::new(Mutex::new(ps));
    let writer: Arc<dyn PlanStateWriter> = Arc::new(DefaultPlanStateWriter::new());
    let tool = ProgressTool::with_writer(Arc::clone(&es), ps, writer);
    (tool, es)
}

/// Helper: create a sample plan markdown file with a progress table.
fn create_sample_plan(path: &str) {
    let content = concat!(
        "# Plan\n",
        "\n",
        "Some description.\n",
        "\n",
        "## Steps\n",
        "\n",
        "- Step 1: do something\n",
        "- Step 2: do something else\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | \u{72b6}\u{6001} | Time | Tokens | Context |\n",
        "|------|------|------|------|--------|---------|\n",
        "| | 1.1 | | | | |\n",
        "| | 2.1 | | | | |\n",
    );
    std::fs::write(path, content).unwrap();
}

#[tokio::test]
async fn test_progress_tool_with_writer_syncs_plan_file() {
    let dir = tempfile::tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");
    create_sample_plan(plan_path.to_str().unwrap());

    let (tool, _es) = make_tool_with_writer(plan_path.to_str().unwrap());
    let ctx = test_ctx();

    // Start step 0 → in_progress
    let result = tool
        .call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await;
    assert!(result.is_ok());

    // Verify the plan file was updated
    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(
        content.contains("[-]"),
        "expected [-] marker after in_progress: {content}"
    );

    // Complete step 0
    let result = tool
        .call(json!({"step_index": 0, "status": "completed"}), &ctx)
        .await;
    assert!(result.is_ok());

    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(
        content.contains("[x]"),
        "expected [x] marker after completed: {content}"
    );
    assert!(
        !content.contains("[-]"),
        "should not contain [-] after completed: {content}"
    );
}

#[tokio::test]
async fn test_progress_tool_with_writer_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let fake_path = dir.path().join("nonexistent_plan.md");

    let (tool, _es) = make_tool_with_writer(fake_path.to_str().unwrap());
    let ctx = test_ctx();

    // Should succeed even though file doesn't exist
    let result = tool
        .call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await;
    assert!(
        result.is_ok(),
        "ProgressTool should not fail when plan file is missing"
    );
}

#[tokio::test]
async fn test_progress_tool_without_writer_no_sync() {
    let (tool, es) = make_tool();
    let ctx = test_ctx();

    // Without writer, should still work normally
    let result = tool
        .call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await;
    assert!(result.is_ok());
    assert_eq!(
        *get_step_status(&es.lock().unwrap(), 0).unwrap(),
        ExecutionStepStatus::InProgress
    );
}

#[tokio::test]
async fn test_progress_tool_with_writer_empty_plan_path() {
    let mut es = ExecutionState::new();
    init_execution_steps(&mut es, vec!["Step 1".into()]);
    let es = Arc::new(Mutex::new(es));
    // plan_file_path is empty by default
    let ps = Arc::new(Mutex::new(PlanState::new()));
    let writer: Arc<dyn PlanStateWriter> = Arc::new(DefaultPlanStateWriter::new());
    let tool = ProgressTool::with_writer(Arc::clone(&es), ps, writer);
    let ctx = test_ctx();

    // Should succeed without attempting to write
    let result = tool
        .call(json!({"step_index": 0, "status": "in_progress"}), &ctx)
        .await;
    assert!(result.is_ok());
}

#[test]
fn test_default_plan_state_writer_marker_mapping() {
    let writer = DefaultPlanStateWriter::new();
    let dir = tempfile::tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");

    // Create plan with step 0
    let content = concat!(
        "# Plan\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Status |\n",
        "|---|---|---|\n",
        "| | 1.1 | detail |\n",
    );
    std::fs::write(&plan_path, content).unwrap();

    // Test InProgress marker
    let mut es = ExecutionState::new();
    es.execution_steps.push(closeclaw_execution::ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::InProgress,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer
        .write_progress_to_plan_file(plan_path.to_str().unwrap(), &es)
        .unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(content.contains("[-]"));
}

#[test]
fn test_default_plan_state_writer_completed_marker() {
    let writer = DefaultPlanStateWriter::new();
    let dir = tempfile::tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");

    let content = concat!(
        "# Plan\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Status |\n",
        "|---|---|---|\n",
        "| | 1.1 | detail |\n",
    );
    std::fs::write(&plan_path, content).unwrap();

    let mut es = ExecutionState::new();
    es.execution_steps.push(closeclaw_execution::ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });
    writer
        .write_progress_to_plan_file(plan_path.to_str().unwrap(), &es)
        .unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(content.contains("[x]"));
}

#[test]
fn test_default_plan_state_writer_failed_marker() {
    let writer = DefaultPlanStateWriter::new();
    let dir = tempfile::tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");

    let content = concat!(
        "# Plan\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Status |\n",
        "|---|---|---|\n",
        "| | 1.1 | detail |\n",
    );
    std::fs::write(&plan_path, content).unwrap();

    let mut es = ExecutionState::new();
    es.execution_steps.push(closeclaw_execution::ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "Step 1".into(),
        error_message: Some("error".into()),
    });
    writer
        .write_progress_to_plan_file(plan_path.to_str().unwrap(), &es)
        .unwrap();
    let content = std::fs::read_to_string(&plan_path).unwrap();
    assert!(content.contains("[!]"));
}

#[test]
fn test_default_plan_state_writer_file_not_found() {
    let writer = DefaultPlanStateWriter::new();
    let es = ExecutionState::new();
    let result = writer.write_progress_to_plan_file("/nonexistent/path/plan.md", &es);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("plan file not found"));
}

#[test]
fn test_default_plan_state_writer_preserves_other_content() {
    let writer = DefaultPlanStateWriter::new();
    let dir = tempfile::tempdir().unwrap();
    let plan_path = dir.path().join("plan.md");

    let content = concat!(
        "# Plan\n",
        "\n",
        "Some description here.\n",
        "\n",
        "## Steps\n",
        "\n",
        "- Step 1\n",
        "- Step 2\n",
        "\n",
        "## Tasks\n",
        "\n",
        "| | Step | Status |\n",
        "|---|---|---|\n",
        "| | 1.1 | detail |\n",
        "| | 2.1 | detail |\n",
        "\n",
        "## Notes\n",
        "\n",
        "Keep this section.\n",
    );
    std::fs::write(&plan_path, content).unwrap();

    let mut es = ExecutionState::new();
    es.execution_steps.push(closeclaw_execution::ExecutionStep {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "Step 1".into(),
        error_message: None,
    });
    es.execution_steps.push(closeclaw_execution::ExecutionStep {
        step_index: 1,
        status: ExecutionStepStatus::InProgress,
        summary: "Step 2".into(),
        error_message: None,
    });
    writer
        .write_progress_to_plan_file(plan_path.to_str().unwrap(), &es)
        .unwrap();

    let result = std::fs::read_to_string(&plan_path).unwrap();
    assert!(result.contains("# Plan"));
    assert!(result.contains("Some description here."));
    assert!(result.contains("## Steps"));
    assert!(result.contains("## Notes"));
    assert!(result.contains("Keep this section."));
    assert!(result.contains("[x]"));
    assert!(result.contains("[-]"));
}
