//! Unit tests for `SessionsSteerTool` and `SessionsKillTool`.
//!
//! Covers early-error (validation) tests and permission engine
//! cross-agent communication checks.

use std::sync::Arc;

use serde_json::json;

use crate::gateway::session_manager::SessionManager;
use crate::gateway::{DmScope, GatewayConfig};
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::rules::RuleSetBuilder;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use crate::tools::builtin::sessions_kill::SessionsKillTool;
use crate::tools::builtin::sessions_steer::SessionsSteerTool;
use crate::tools::{Tool, ToolCallError, ToolContext};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_gateway_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
        ..Default::default()
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &test_gateway_config(),
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

/// Build a `PermissionEngine` with an empty RuleSet (all defaults apply).
fn make_permission_engine() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn ctx_with_session(session_id: &str) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()),
        call_id: None,
        session: None,
    }
}

fn ctx_without_session() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    }
}

// ---------------------------------------------------------------------------
// Steer: missing childId
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_missing_child_id() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({"task": "something"}), &ctx).await;

    let err = result.expect_err("steer should fail when childId is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("childId"),
                "error should mention childId, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Steer: missing task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_missing_task() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({"childId": "some-id"}), &ctx).await;

    let err = result.expect_err("steer should fail when task is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("task"),
                "error should mention task, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Kill: missing childId
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_missing_child_id() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsKillTool::new(mgr, pe);
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({}), &ctx).await;

    let err = result.expect_err("kill should fail when childId is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("childId"),
                "error should mention childId, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Steer: no session_id in context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_no_session_id_in_context() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
    let ctx = ctx_without_session();

    let result = tool
        .call(json!({"childId": "some-id", "task": "something"}), &ctx)
        .await;

    let err = result.expect_err("steer should fail when session_id is None");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("session_id"),
                "error should mention session_id, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Kill: no session_id in context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_no_session_id_in_context() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsKillTool::new(mgr, pe);
    let ctx = ctx_without_session();

    let result = tool.call(json!({"childId": "some-id"}), &ctx).await;

    let err = result.expect_err("kill should fail when session_id is None");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("session_id"),
                "error should mention session_id, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Steer: child session not found (ownership + permission path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_child_not_found() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
    let ctx = ctx_with_session("parent-x");

    let result = tool
        .call(
            json!({"childId": "nonexistent-child", "task": "redo"}),
            &ctx,
        )
        .await;

    let err = result.expect_err("steer should fail when child not found");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("child session not found"),
                "error should mention ownership, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Kill: child session not found (ownership + permission path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_child_not_found() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsKillTool::new(mgr, pe);
    let ctx = ctx_with_session("parent-x");

    let result = tool
        .call(json!({"childId": "nonexistent-child"}), &ctx)
        .await;

    let err = result.expect_err("kill should fail when child not found");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("child session not found"),
                "error should mention ownership, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}
