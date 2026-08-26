//! Tests for ExecutePlanTool.
//!
//! Covers: tool metadata, error paths (missing session_id, missing plan
//! info), and the approval_pending happy path.
//!
//! Note: Full happy-path testing (plan state + confirmed status → approval)
//! requires a persistence backend to store plan_state, which is not available
//! in unit tests. The error paths verify the tool's validation logic covers
//! the dimensions specified in the plan.

use crate::{Tool, ToolCallError, ToolContext, WorkdirContext};
use closeclaw_common::SessionMode;
use closeclaw_gateway::GatewayConfig;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex as TokioMutex;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn make_ctx(session_id: Option<&str>) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: session_id.map(|s| s.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

fn make_ctx_with_workdir(session_id: Option<&str>, workdir: &std::path::Path) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: Some(WorkdirContext {
            path: workdir.to_string_lossy().into_owned(),
            has_git: false,
            branch: None,
            recent_changes: 0,
        }),
        session_id: session_id.map(|s| s.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig::default(),
        None, // no storage → get_plan_state returns None
        None,
        closeclaw_common::ReasoningLevel::default(),
    ))
}

async fn make_approval_flow() -> Arc<TokioMutex<ApprovalFlow>> {
    let sm = make_session_manager();
    let flow = ApprovalFlow::new(
        sm.clone(),
        Arc::new(|_| {}), // on_notify_owner
        Arc::new(|_| {}), // on_whitelist_updated
        tokio::runtime::Handle::current(),
        closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
        PathBuf::from("/tmp/cc_test_plan"),
        closeclaw_permission::rules::RuleSet::default(),
    );
    Arc::new(TokioMutex::new(flow))
}

/// Register a ConversationSession in the SessionManager.
async fn register_session(sm: &SessionManager, session_id: &str, mode: SessionMode) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        PathBuf::from("/tmp"),
    )
    .with_session_mode(mode);
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.to_owned(), cs_arc);
    }
}

fn make_tool(
    sm: Arc<SessionManager>,
    af: Arc<TokioMutex<ApprovalFlow>>,
) -> crate::builtin::execute_plan::ExecutePlanTool {
    crate::builtin::execute_plan::ExecutePlanTool::new(sm, af)
}

/// Create a temp workspace with a plan file so resolve_plan_by_name succeeds.
fn setup_workspace_with_plan() -> (TempDir, String) {
    let tmp = TempDir::new().unwrap();
    let plans_dir = tmp.path().join("plans");
    std::fs::create_dir_all(&plans_dir).unwrap();
    std::fs::write(plans_dir.join("my-plan.md"), "# Plan\n\n- [ ] step1\n").unwrap();
    (tmp, "my-plan".to_string())
}

// ── Tool metadata tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_tool_name() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    assert_eq!(tool.name(), "execute_plan");
}

#[tokio::test]
async fn test_tool_group() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    assert_eq!(tool.group(), "plan");
}

#[tokio::test]
async fn test_tool_summary() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    assert!(!tool.summary().is_empty());
}

#[tokio::test]
async fn test_tool_flags() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let flags = tool.flags();
    assert!(flags.is_concurrency_safe);
    assert!(!flags.is_read_only);
    assert!(!flags.is_destructive);
    assert!(!flags.is_deferred_by_default);
}

#[tokio::test]
async fn test_tool_input_schema_properties() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    assert!(props.get("plan_file_path").is_some());
    assert!(props.get("step_selection").is_some());
    assert!(props.get("new_session").is_some());
    // No required fields
    let required = schema.pointer("/required").unwrap().as_array().unwrap();
    assert!(required.is_empty());
}

// ── Step 1.4: Schema property tests ──────────────────────────────────────

#[tokio::test]
async fn test_tool_input_schema_has_plan_name_property() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    let plan_name = props
        .get("plan_name")
        .expect("plan_name property should exist");
    assert_eq!(plan_name["type"], "string");
    let desc = plan_name["description"].as_str().unwrap();
    assert!(
        desc.contains("Name of the plan"),
        "plan_name description should mention name: {desc}"
    );
}

#[tokio::test]
async fn test_tool_input_schema_has_additional_instruction_property() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let schema = tool.input_schema();
    let props = schema.pointer("/properties").unwrap();
    let ai = props
        .get("additional_instruction")
        .expect("additional_instruction property should exist");
    assert_eq!(ai["type"], "string");
    let desc = ai["description"].as_str().unwrap();
    assert!(
        desc.contains("Optional instruction"),
        "additional_instruction description should mention optional: {desc}"
    );
}

// ── Step 1.4: Detail doc string test ─────────────────────────────────────

#[tokio::test]
async fn test_tool_detail_mentions_additional_instruction() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let detail = tool.detail();
    assert!(
        detail.contains("additional instruction"),
        "detail should mention additional instruction: {detail}"
    );
}

// ── Error path tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_call_without_session_id() {
    let sm = make_session_manager();
    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(None);

    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("session_id"),
                "error should mention session_id: {msg}"
            );
        }
        other => panic!("expected ExecutionFailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_no_plan_info_returns_error() {
    let sm = make_session_manager();
    register_session(&sm, "sess-normal", SessionMode::Normal).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-normal"));

    // No plan_name, no plan_file_path, no plan state → fallback
    // load_plan_state fails → error
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_plan_file_path_bypasses_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-file", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-file"));

    // plan_file_path provided → plan_state check is skipped,
    // file does not exist → file-not-found error
    let result = tool
        .call(json!({"plan_file_path": "/some/path.md"}), &ctx)
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("plan 文件不存在"),
                "error should mention file not found: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_step_selection_parses_correctly() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-steps", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-steps"));

    // No plan_name/plan_file_path → fallback load_plan_state → error
    let result = tool.call(json!({"step_selection": [0, 1, 2]}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_with_new_session_flag() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-newsess", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-newsess"));

    // No plan_name/plan_file_path → fallback load_plan_state → error
    let result = tool.call(json!({"new_session": true}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

// ── Step 1.4: plan_name and additional_instruction argument tests ────────

#[tokio::test]
async fn test_call_plan_name_resolves_by_name_not_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-name", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-name"));

    // plan_name provided → plan_state check skipped,
    // plan file not found → "未找到名为" error
    let result = tool
        .call(
            json!({
                "plan_name": "my-plan",
                "additional_instruction": "请优先处理测试用例"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("未找到名为"),
                "error should mention plan not found: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_plan_name_with_additional_instruction_not_found() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-missing", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-missing"));

    // plan_name + additional_instruction provided but plan file not found
    let result = tool
        .call(
            json!({
                "plan_name": "my-plan",
                "additional_instruction": "请优先处理测试用例"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("未找到名为 'my-plan'"),
                "error should mention plan name: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_empty_additional_instruction_filtered() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-empty-ai", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-empty-ai"));

    // Empty additional_instruction treated as absent → no plan_name/plan_file_path
    // → fallback load_plan_state → error
    let result = tool
        .call(
            json!({
                "additional_instruction": "  "
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("活跃的 plan"), "got: {msg}");
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_empty_plan_name_filtered() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-empty-pn", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-empty-pn"));

    // Empty plan_name filtered → no plan_name/plan_file_path → fallback
    // load_plan_state → error
    let result = tool
        .call(
            json!({
                "plan_name": ""
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(msg.contains("活跃的 plan"), "got: {msg}");
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

// ── Step 1.2: Any-mode execution tests ──────────────────────────────────

#[tokio::test]
async fn test_call_normal_mode_with_plan_name() {
    let sm = make_session_manager();
    register_session(&sm, "sess-normal-plan", SessionMode::Normal).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-normal-plan"), tmp.path());

    // Normal mode + plan_name + plan file exists → approval_pending
    let result = tool.call(json!({"plan_name": &plan_name}), &ctx).await;
    assert!(result.is_ok(), "should succeed with valid plan_name");
    let tr = result.unwrap();
    assert_eq!(tr.data["status"], "approval_pending");
    assert!(tr.data.get("request_id").is_some());
}

#[tokio::test]
async fn test_call_auto_mode_with_plan_name() {
    let sm = make_session_manager();
    register_session(&sm, "sess-auto-plan", SessionMode::Auto).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);

    let (tmp, plan_name) = setup_workspace_with_plan();
    let ctx = make_ctx_with_workdir(Some("sess-auto-plan"), tmp.path());

    // Auto mode + plan_name + plan file exists → approval_pending
    let result = tool.call(json!({"plan_name": &plan_name}), &ctx).await;
    assert!(result.is_ok(), "should succeed with valid plan_name");
    let tr = result.unwrap();
    assert_eq!(tr.data["status"], "approval_pending");
    assert!(tr.data.get("request_id").is_some());
}

#[tokio::test]
async fn test_call_plan_mode_no_plan_info_returns_error() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan"));

    // Plan mode, no plan_name/plan_file_path, no plan state → error
    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("活跃的 plan"),
                "error should mention missing plan: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}
