//! Tests for ExecutePlanTool.
//!
//! Covers: tool metadata, error paths (missing session_id, non-Plan Mode,
//! missing plan state), and the happy path (returns approval_pending).
//!
//! Note: Full happy-path testing (plan state + confirmed status → approval)
//! requires a persistence backend to store plan_state, which is not available
//! in unit tests. The error paths verify the tool's validation logic covers
//! the dimensions specified in the plan.

use crate::{Tool, ToolCallError, ToolContext};
use closeclaw_common::SessionMode;
use closeclaw_gateway::GatewayConfig;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
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
    use std::path::PathBuf;
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
async fn test_call_not_in_plan_mode() {
    let sm = make_session_manager();
    register_session(&sm, "sess-auto", SessionMode::Auto).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-auto"));

    let result = tool.call(json!({}), &ctx).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("Plan Mode"),
                "error should mention Plan Mode: {msg}"
            );
        }
        other => panic!("expected InvalidArgs, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_call_plan_mode_no_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan"));

    // No plan state (storage is None) → error
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
async fn test_call_plan_mode_with_plan_file_path_still_needs_plan_state() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-file", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-file"));

    // Even with explicit plan_file_path, plan_state is checked first
    let result = tool
        .call(json!({"plan_file_path": "/some/path.md"}), &ctx)
        .await;
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
async fn test_call_with_step_selection_parses_correctly() {
    let sm = make_session_manager();
    register_session(&sm, "sess-plan-steps", SessionMode::Plan).await;

    let af = make_approval_flow().await;
    let tool = make_tool(sm, af);
    let ctx = make_ctx(Some("sess-plan-steps"));

    // Step selection is parsed but plan state check fails first
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
