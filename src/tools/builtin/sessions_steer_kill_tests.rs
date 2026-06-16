//! Unit tests for `SessionsSteerTool` and `SessionsKillTool`.
//!
//! Covers early-error (validation) tests and permission engine
//! cross-agent communication checks.

use std::sync::Arc;

use serde_json::json;

use crate::gateway::session_manager::{ChildSessionInfo, SessionManager, SpawnMode};
use crate::gateway::{DmScope, GatewayConfig, Session};
use crate::llm::session::ConversationSession;
use crate::permission::actions::ActionBuilder;
use crate::permission::engine::engine_eval::PermissionEngine;
use crate::permission::engine::engine_types::{Effect, Rule};
use crate::permission::rules::{RuleBuilder, RuleSetBuilder};
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
// Steer: missing sessionId
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_missing_child_id() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
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

// ---------------------------------------------------------------------------
// Steer: missing task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_missing_task() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsSteerTool::new(mgr, pe);
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

// ---------------------------------------------------------------------------
// Kill: missing sessionId
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_missing_child_id() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsKillTool::new(mgr, pe);
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

// ---------------------------------------------------------------------------
// Kill: no session_id in context
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_no_session_id_in_context() {
    let mgr = make_session_manager();
    let pe = make_permission_engine();
    let tool = SessionsKillTool::new(mgr, pe);
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
            json!({"sessionId": "nonexistent-child", "task": "redo"}),
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
        .call(json!({"sessionId": "nonexistent-child"}), &ctx)
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

// ===========================================================================
// Step 1.5: Permission engine cross-agent communication tests
// ===========================================================================

/// Helper: build a `SessionManager` with a parent session registered
/// in `sessions` (for agent_id lookup), a parent `ConversationSession`
/// in `conversation_sessions`, and a child session registered in
/// `children` (for ownership validation) with its own
/// `ConversationSession` (required by steer/kill operations).
async fn setup_parent_child(
    mgr: &SessionManager,
    parent_session_id: &str,
    parent_agent_id: &str,
    child_session_id: &str,
    child_agent_id: &str,
) {
    // Register parent session (for get_chat_id lookup).
    mgr.sessions.write().await.insert(
        parent_session_id.to_string(),
        Session {
            id: parent_session_id.to_string(),
            agent_id: parent_agent_id.to_string(),
            channel: "test-channel".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    let tmp = tempfile::tempdir().unwrap();
    // Register a ConversationSession for the parent.
    let parent_cs = ConversationSession::new(
        parent_session_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    mgr.conversation_sessions.write().await.insert(
        parent_session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(parent_cs)),
    );

    // Register a ConversationSession for the child (required by
    // steer_child / kill_child which call get_conversation_session).
    let child_cs = ConversationSession::new(
        child_session_id.to_string(),
        "test-model".to_string(),
        tmp.path().to_path_buf(),
    );
    mgr.conversation_sessions.write().await.insert(
        child_session_id.to_string(),
        Arc::new(tokio::sync::RwLock::new(child_cs)),
    );

    // Register child in the children tracking table.
    mgr.register_child(
        parent_session_id,
        ChildSessionInfo {
            session_id: child_session_id.to_string(),
            parent_session_id: parent_session_id.to_string(),
            agent_id: child_agent_id.to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;
}

/// Helper: build a `PermissionEngine` with a specific inter_agent
/// default effect and optional explicit allow rule for a given agent
/// communicating to a target agent.
fn make_permission_engine_with_rules(
    default_inter_agent: Effect,
    allow_rules: Vec<Rule>,
) -> Arc<PermissionEngine> {
    let mut builder = RuleSetBuilder::new().default_inter_agent(default_inter_agent);
    for rule in allow_rules {
        builder = builder.rule(rule);
    }
    Arc::new(PermissionEngine::new_with_default_data_root(
        builder.build().unwrap(),
    ))
}

/// Helper: build an allow rule for inter-agent communication from
/// `from_agent` to `to_agent`.
fn inter_agent_allow_rule(from_agent: &str, to_agent: &str) -> Rule {
    RuleBuilder::new()
        .name(format!("allow-{}-to-{}", from_agent, to_agent))
        .subject_agent(from_agent)
        .allow()
        .action(
            ActionBuilder::inter_agent()
                .with_agents(vec![to_agent.to_string()])
                .build()
                .unwrap(),
        )
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Steer: permission allowed → steer succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_permission_allowed() {
    let mgr = make_session_manager();

    // Default inter_agent = Allow → permission engine allows the operation.
    let pe = make_permission_engine_with_rules(Effect::Allow, vec![]);
    let tool = SessionsSteerTool::new(mgr.clone(), pe);

    let parent_id = "parent-perm-ok";
    let child_id = "child-perm-ok";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool
        .call(json!({"sessionId": child_id, "task": "new task"}), &ctx)
        .await;

    let tool_result = result.expect("steer should succeed when permission is allowed");
    assert_eq!(tool_result.data["child_id"], child_id);
    assert_eq!(tool_result.data["task"], "new task");
}

// ---------------------------------------------------------------------------
// Steer: permission denied → steer fails with permission error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_permission_denied() {
    let mgr = make_session_manager();

    // Default inter_agent = Deny, no allow rule → permission engine denies.
    let pe = make_permission_engine_with_rules(Effect::Deny, vec![]);
    let tool = SessionsSteerTool::new(mgr.clone(), pe);

    let parent_id = "parent-perm-deny";
    let child_id = "child-perm-deny";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool
        .call(json!({"sessionId": child_id, "task": "new task"}), &ctx)
        .await;

    let err = result.expect_err("steer should fail when permission is denied");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("permission denied"),
                "error should mention permission denied, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Kill: permission allowed → kill succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_permission_allowed() {
    let mgr = make_session_manager();

    // Default inter_agent = Allow → permission engine allows the operation.
    let pe = make_permission_engine_with_rules(Effect::Allow, vec![]);
    let tool = SessionsKillTool::new(mgr.clone(), pe);

    let parent_id = "parent-kill-perm-ok";
    let child_id = "child-kill-perm-ok";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool.call(json!({"sessionId": child_id}), &ctx).await;

    let tool_result = result.expect("kill should succeed when permission is allowed");
    assert_eq!(tool_result.data["child_id"], child_id);
    assert_eq!(tool_result.data["status"], "killed");
}

// ---------------------------------------------------------------------------
// Kill: permission denied → kill fails with permission error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_permission_denied() {
    let mgr = make_session_manager();

    // Default inter_agent = Deny, no allow rule → permission engine denies.
    let pe = make_permission_engine_with_rules(Effect::Deny, vec![]);
    let tool = SessionsKillTool::new(mgr.clone(), pe);

    let parent_id = "parent-kill-perm-deny";
    let child_id = "child-kill-perm-deny";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool.call(json!({"sessionId": child_id}), &ctx).await;

    let err = result.expect_err("kill should fail when permission is denied");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("permission denied"),
                "error should mention permission denied, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Steer: explicit allow rule overrides default deny
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_steer_explicit_allow_overrides_default_deny() {
    let mgr = make_session_manager();

    // Default inter_agent = Deny, but explicit allow rule for parent-agent → child-agent.
    let pe = make_permission_engine_with_rules(
        Effect::Deny,
        vec![inter_agent_allow_rule("parent-agent", "child-agent")],
    );
    let tool = SessionsSteerTool::new(mgr.clone(), pe);

    let parent_id = "parent-explicit-ok";
    let child_id = "child-explicit-ok";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool
        .call(json!({"sessionId": child_id, "task": "steer task"}), &ctx)
        .await;

    let tool_result =
        result.expect("steer should succeed when explicit allow rule overrides default deny");
    assert_eq!(tool_result.data["child_id"], child_id);
}

// ---------------------------------------------------------------------------
// Kill: explicit allow rule overrides default deny
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_explicit_allow_overrides_default_deny() {
    let mgr = make_session_manager();

    // Default inter_agent = Deny, but explicit allow rule for parent-agent → child-agent.
    let pe = make_permission_engine_with_rules(
        Effect::Deny,
        vec![inter_agent_allow_rule("parent-agent", "child-agent")],
    );
    let tool = SessionsKillTool::new(mgr.clone(), pe);

    let parent_id = "parent-kill-explicit-ok";
    let child_id = "child-kill-explicit-ok";
    setup_parent_child(&mgr, parent_id, "parent-agent", child_id, "child-agent").await;

    let ctx = ToolContext {
        agent_id: "parent-agent".to_string(),
        workdir: None,
        session_id: Some(parent_id.to_string()),
        call_id: None,
        session: None,
    };

    let result = tool.call(json!({"sessionId": child_id}), &ctx).await;

    let tool_result =
        result.expect("kill should succeed when explicit allow rule overrides default deny");
    assert_eq!(tool_result.data["child_id"], child_id);
}
