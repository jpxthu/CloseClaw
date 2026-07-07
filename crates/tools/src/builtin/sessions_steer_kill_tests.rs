//! Unit tests for `SessionsSteerTool` and `SessionsKillTool`.
//!
//! Covers:
//! - Parameter validation (sessionId, task, context)
//! - Mode gating (steer rejects mode=run, kill accepts both)
//! - Permission engine integration (allowed / denied)
//! - Successful steer and kill paths

use std::sync::Arc;

use serde_json::json;

use crate::builtin::sessions_kill::SessionsKillTool;
use crate::builtin::sessions_steer::SessionsSteerTool;
use crate::{Tool, ToolCallError, ToolContext};
use closeclaw_gateway::session_manager::{ChildSessionInfo, SpawnMode};
use closeclaw_gateway::{DmScope, GatewayConfig, Message, Session, SessionManager};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{Action, Effect, Rule, Subject};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use tokio::sync::RwLock;

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

/// PermissionEngine with default empty ruleset — all inter-agent denied.
fn make_permission_engine_deny_all() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

/// PermissionEngine that allows inter-agent messages for "test-agent".
///
/// The engine uses a two-phase merge: both agent-phase AND user-phase
/// must return Allowed. We provide matching rules for both phases.
fn make_permission_engine_allow_inter_agent() -> Arc<PermissionEngine> {
    let agent_rule = Rule {
        name: "allow-test-agent-inter-agent-phase".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::InterAgent {
            agents: vec!["*".to_string()],
        }],
        template: None,
        priority: 10,
    };
    // User-phase rule: allow with empty user_id (matches Bare requests)
    let user_rule = Rule {
        name: "allow-test-agent-inter-user-phase".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "".to_string(),
            agent: "test-agent".to_string(),
            user_match: Default::default(),
            agent_match: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::InterAgent {
            agents: vec!["*".to_string()],
        }],
        template: None,
        priority: 10,
    };
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new()
            .rule(agent_rule)
            .rule(user_rule)
            .build()
            .unwrap(),
    ))
}

/// Mock ApprovalFlow for tests — never enqueues anything.
fn make_mock_approval_flow() -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::new(make_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
    )))
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

/// Register a parent session via `find_or_create` so it appears in both
/// `sessions` and `conversation_sessions` (needed by steer_child / kill_child).
async fn setup_parent_session(mgr: &SessionManager, parent_id: &str) {
    let msg = Message {
        id: format!("msg-{}", parent_id),
        from: "user".to_string(),
        to: "test-agent".to_string(),
        content: "hi".to_string(),
        channel: "test-channel".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
    };
    let session_id = mgr
        .find_or_create("test-channel", &msg, None)
        .await
        .expect("find_or_create should succeed");
    // The created session_id may differ from parent_id; insert under the
    // expected parent_id so validate_child_ownership matches ctx.session_id.
    if session_id != parent_id {
        let cs = mgr
            .conversation_sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .expect("find_or_create should register conversation_session");
        mgr.conversation_sessions
            .write()
            .await
            .insert(parent_id.to_string(), cs);
        let sess = mgr
            .sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .expect("find_or_create should register session");
        mgr.sessions.write().await.insert(
            parent_id.to_string(),
            Session {
                id: parent_id.to_string(),
                ..sess
            },
        );
    }
}

/// Register a child session in all three tracking structures:
/// `conversation_sessions`, `sessions`, and the `children` SpawnTree.
async fn setup_child_session(
    mgr: &SessionManager,
    parent_id: &str,
    child_id: &str,
    mode: SpawnMode,
) {
    // 1. ConversationSession (needed by steer_child / kill_child)
    let cs = ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
    );
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), Arc::new(RwLock::new(cs)));

    // 2. Session entry (needed by kill_child cleanup)
    mgr.sessions.write().await.insert(
        child_id.to_string(),
        Session {
            id: child_id.to_string(),
            agent_id: "child-agent".to_string(),
            channel: "spawn".to_string(),
            created_at: 0,
            depth: 1,
        },
    );

    // 3. Children SpawnTree (needed by validate_child_ownership)
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode,
        },
    )
    .await;
}

// ===========================================================================
// Steer: parameter validation
// ===========================================================================

#[tokio::test]
async fn test_steer_missing_session_id() {
    let mgr = make_session_manager();
    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({"task": "something"}), &ctx).await;

    let err = result.expect_err("steer should fail when sessionId is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("sessionId"),
                "error should mention sessionId, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_steer_missing_task() {
    let mgr = make_session_manager();
    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({"sessionId": "some-id"}), &ctx).await;

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

#[tokio::test]
async fn test_steer_no_session_id_in_context() {
    let mgr = make_session_manager();
    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_without_session();

    let result = tool
        .call(json!({"sessionId": "some-id", "task": "something"}), &ctx)
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

// ===========================================================================
// Kill: parameter validation
// ===========================================================================

#[tokio::test]
async fn test_kill_missing_session_id() {
    let mgr = make_session_manager();
    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-x");

    let result = tool.call(json!({}), &ctx).await;

    let err = result.expect_err("kill should fail when sessionId is missing");
    match err {
        ToolCallError::InvalidArgs(msg) => {
            assert!(
                msg.contains("sessionId"),
                "error should mention sessionId, got: {}",
                msg
            );
        }
        other => panic!("expected InvalidArgs, got {:?}", other),
    }
}

#[tokio::test]
async fn test_kill_no_session_id_in_context() {
    let mgr = make_session_manager();
    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_without_session();

    let result = tool.call(json!({"sessionId": "some-id"}), &ctx).await;

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

// ===========================================================================
// Steer: ownership / not found
// ===========================================================================

#[tokio::test]
async fn test_steer_child_not_found() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-nf").await;

    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-nf");

    let result = tool
        .call(
            json!({"sessionId": "nonexistent-child", "task": "do something"}),
            &ctx,
        )
        .await;

    let err = result.expect_err("steer should fail for unknown child");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("not found") || msg.contains("not owned"),
                "error should indicate child not found, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ===========================================================================
// Kill: ownership / not found
// ===========================================================================

#[tokio::test]
async fn test_kill_child_not_found() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-knf").await;

    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-knf");

    let result = tool
        .call(json!({"sessionId": "nonexistent-child"}), &ctx)
        .await;

    let err = result.expect_err("kill should fail for unknown child");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("not found") || msg.contains("not owned"),
                "error should indicate child not found, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ===========================================================================
// Steer: mode gating — rejects mode=run
// ===========================================================================

#[tokio::test]
async fn test_steer_rejects_mode_run() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-run").await;
    setup_child_session(&mgr, "parent-run", "child-run", SpawnMode::Run).await;

    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_allow_inter_agent(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-run");

    let result = tool
        .call(json!({"sessionId": "child-run", "task": "new task"}), &ctx)
        .await;

    let err = result.expect_err("steer should reject mode=run child");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("mode") || msg.contains("session"),
                "error should mention mode restriction, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ===========================================================================
// Steer: permission denied
// ===========================================================================

#[tokio::test]
async fn test_steer_permission_denied() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-pd").await;
    setup_child_session(&mgr, "parent-pd", "child-pd", SpawnMode::Session).await;

    // Default ruleset denies everything
    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-pd");

    let result = tool
        .call(json!({"sessionId": "child-pd", "task": "new task"}), &ctx)
        .await;

    let err = result.expect_err("steer should fail when permission denied");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("denied") || msg.contains("permission"),
                "error should mention permission denial, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ===========================================================================
// Steer: success path — mode=session + permission allowed
// ===========================================================================

#[tokio::test]
async fn test_steer_success_mode_session_permission_allowed() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-ok").await;
    setup_child_session(&mgr, "parent-ok", "child-ok", SpawnMode::Session).await;

    let tool = SessionsSteerTool::new(
        mgr,
        make_permission_engine_allow_inter_agent(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-ok");

    let result = tool
        .call(
            json!({"sessionId": "child-ok", "task": "inject this"}),
            &ctx,
        )
        .await;

    let output = result.expect("steer should succeed");
    assert_eq!(
        output.data["child_id"], "child-ok",
        "returned child_id should match"
    );
    assert_eq!(
        output.data["task"], "inject this",
        "returned task should match"
    );
}

// ===========================================================================
// Kill: permission denied
// ===========================================================================

#[tokio::test]
async fn test_kill_permission_denied() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-kd").await;
    setup_child_session(&mgr, "parent-kd", "child-kd", SpawnMode::Session).await;

    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_deny_all(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-kd");

    let result = tool.call(json!({"sessionId": "child-kd"}), &ctx).await;

    let err = result.expect_err("kill should fail when permission denied");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("denied") || msg.contains("permission"),
                "error should mention permission denial, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ===========================================================================
// Kill: success path — permission allowed + mode=session
// ===========================================================================

#[tokio::test]
async fn test_kill_success_mode_session_permission_allowed() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-ks").await;
    setup_child_session(&mgr, "parent-ks", "child-ks", SpawnMode::Session).await;

    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_allow_inter_agent(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-ks");

    let result = tool.call(json!({"sessionId": "child-ks"}), &ctx).await;

    let output = result.expect("kill should succeed for mode=session");
    assert_eq!(
        output.data["child_id"], "child-ks",
        "returned child_id should match"
    );
    assert_eq!(
        output.data["status"], "killed",
        "returned status should be 'killed'"
    );
}

// ===========================================================================
// Kill: success path — permission allowed + mode=run (kill does NOT check mode)
// ===========================================================================

#[tokio::test]
async fn test_kill_success_mode_run_permission_allowed() {
    let mgr = make_session_manager();
    setup_parent_session(&mgr, "parent-kr").await;
    setup_child_session(&mgr, "parent-kr", "child-kr", SpawnMode::Run).await;

    let tool = SessionsKillTool::new(
        mgr,
        make_permission_engine_allow_inter_agent(),
        make_mock_approval_flow(),
    );
    let ctx = ctx_with_session("parent-kr");

    let result = tool.call(json!({"sessionId": "child-kr"}), &ctx).await;

    let output = result.expect("kill should succeed for mode=run");
    assert_eq!(
        output.data["child_id"], "child-kr",
        "returned child_id should match"
    );
    assert_eq!(
        output.data["status"], "killed",
        "returned status should be 'killed'"
    );
}
