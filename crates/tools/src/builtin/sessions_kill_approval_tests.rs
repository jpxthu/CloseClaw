//! Approval flow routing tests for SessionsKillTool.
//!
//! Covers three paths:
//! 1. allow — permission passes, child killed successfully
//! 2. deny + enqueue success — returns approval_pending
//! 3. deny + enqueue failure (duplicate) — fallback to ExecutionFailed

use std::sync::Arc;

use serde_json::json;

use crate::builtin::sessions_kill::SessionsKillTool;
use crate::{Tool, ToolCallError, ToolContext};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use closeclaw_gateway::session_manager::{ChildSessionInfo, SpawnMode};
use closeclaw_gateway::{DmScope, GatewayConfig, Message, Session, SessionManager};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{
    Action, Caller, Effect, PermissionRequest, PermissionRequestBody, PermissionResponse, Rule,
    Subject,
};
use closeclaw_permission::rules::RuleSetBuilder;
use closeclaw_session::bootstrap::BootstrapMode;
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use std::collections::HashMap;
use tokio::sync::RwLock;

struct MockModeQuery {
    modes: HashMap<String, SessionMode>,
}

impl MockModeQuery {
    fn new() -> Self {
        Self {
            modes: HashMap::new(),
        }
    }

    fn with_mode(mut self, agent_id: &str, mode: SessionMode) -> Self {
        self.modes.insert(agent_id.to_string(), mode);
        self
    }
}

impl SessionModeQuery for MockModeQuery {
    fn get_session_mode(&self, agent_id: &str) -> Option<SessionMode> {
        self.modes.get(agent_id).copied()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deny_all_engine() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn allow_inter_agent_engine() -> Arc<PermissionEngine> {
    let agent_rule = Rule {
        name: "allow-inter-agent-phase".to_string(),
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
    let user_rule = Rule {
        name: "allow-inter-user-phase".to_string(),
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

fn make_approval_flow() -> Arc<tokio::sync::Mutex<ApprovalFlow>> {
    Arc::new(tokio::sync::Mutex::new(ApprovalFlow::new(
        Arc::clone(&make_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
    )))
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: DmScope::default(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn make_ctx(session_id: &str) -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: Some(session_id.to_string()),
        call_id: None,
        session: None,
        session_mode: None,
    }
}

/// Register a parent and child session so validate_child_ownership succeeds.
async fn setup_sessions(mgr: &SessionManager, parent_id: &str, child_id: &str) {
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

    let cs = ConversationSession::new(
        child_id.to_string(),
        "test-model".to_string(),
        std::env::temp_dir(),
    );
    mgr.conversation_sessions
        .write()
        .await
        .insert(child_id.to_string(), Arc::new(RwLock::new(cs)));
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
    mgr.register_child(
        parent_id,
        ChildSessionInfo {
            session_id: child_id.to_string(),
            parent_session_id: parent_id.to_string(),
            agent_id: "child-agent".to_string(),
            depth: 1,
            mode: SpawnMode::Session,
        },
    )
    .await;
}

/// Engine with Auto mode enabled — will return `ApprovalRequired` for
/// operations where `assess_risk_level` returns High/Critical.
/// Uses `Action::All` so that any request type (including FileOp) is allowed,
/// allowing the auto mode risk gate to trigger on high-risk requests.
fn auto_mode_allow_engine() -> Arc<PermissionEngine> {
    let agent_rule = Rule {
        name: "allow-inter-agent-phase".to_string(),
        subject: Subject::AgentOnly {
            agent: "test-agent".to_string(),
            match_type: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::All],
        template: None,
        priority: 10,
    };
    let user_rule = Rule {
        name: "allow-inter-user-phase".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "".to_string(),
            agent: "test-agent".to_string(),
            user_match: Default::default(),
            agent_match: Default::default(),
        },
        effect: Effect::Allow,
        actions: vec![Action::All],
        template: None,
        priority: 10,
    };
    Arc::new(
        PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new()
                .rule(agent_rule)
                .rule(user_rule)
                .build()
                .unwrap(),
        )
        .with_session_mode_query(Arc::new(
            MockModeQuery::new().with_mode("test-agent", SessionMode::Auto),
        )),
    )
}

// ---------------------------------------------------------------------------
// Path 1: allow — permission passes, child killed successfully
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_approval_allow_path() {
    let mgr = make_session_manager();
    setup_sessions(&mgr, "parent-allow", "child-allow").await;
    let tool = SessionsKillTool::new(mgr, allow_inter_agent_engine(), make_approval_flow());
    let ctx = make_ctx("parent-allow");

    let result = tool.call(json!({"sessionId": "child-allow"}), &ctx).await;
    let output = result.expect("allow path should succeed");
    assert_eq!(output.data["status"], "killed");
}

// ---------------------------------------------------------------------------
// Path 2: deny + enqueue success → approval_pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_approval_deny_enqueue_success() {
    let mgr = make_session_manager();
    setup_sessions(&mgr, "parent-deny", "child-deny").await;
    let tool = SessionsKillTool::new(mgr, deny_all_engine(), make_approval_flow());
    let ctx = make_ctx("parent-deny");

    let result = tool.call(json!({"sessionId": "child-deny"}), &ctx).await;
    let output = result.expect("deny+enqueue should return Ok");
    assert_eq!(
        output.data["status"], "approval_pending",
        "should return approval_pending"
    );
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id"
    );
}

// ---------------------------------------------------------------------------
// Path 3: deny + enqueue failure (duplicate) → fallback to ExecutionFailed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_kill_approval_deny_enqueue_fallback() {
    let flow = make_approval_flow();
    // Pre-enqueue a matching denial so the tool hits duplicate detection.
    let caller = Caller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::InterAgentMsg {
        from: "test-agent".to_string(),
        to: "child-agent".to_string(),
    };
    {
        let mut f = flow.lock().await;
        f.submit_denial(&caller, &body, RiskLevel::Medium, "", false)
            .expect("first enqueue should succeed");
    }

    let mgr = make_session_manager();
    setup_sessions(&mgr, "parent-fb", "child-fb").await;
    let tool = SessionsKillTool::new(mgr, deny_all_engine(), Arc::clone(&flow));
    let ctx = make_ctx("parent-fb");

    let result = tool.call(json!({"sessionId": "child-fb"}), &ctx).await;
    let err = result.expect_err("fallback should return error");
    match err {
        ToolCallError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("denied"),
                "error should mention denied, got: {}",
                msg
            );
        }
        other => panic!("expected ExecutionFailed, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Path 4: ApprovalRequired → approval flow routing
// ---------------------------------------------------------------------------

/// Verify that when the permission engine returns `ApprovalRequired`, the
/// response is properly routed through the approval flow, returning
/// `approval_pending` with a request_id.
///
/// InterAgentMsg is always Low risk in the current risk assessment, so
/// `ApprovalRequired` cannot be triggered through normal tool execution.
/// This test directly exercises the approval routing mechanism by calling
/// `evaluate()` with a high-risk request and manually routing the response
/// through the approval flow — the same mechanism the tool uses.
#[tokio::test]
async fn test_kill_approval_required_routes_through_approval_flow() {
    let flow = make_approval_flow();
    let engine = auto_mode_allow_engine();
    let ctx = make_ctx("parent-auto");

    // 1. Verify the engine returns ApprovalRequired for a high-risk request.
    //    Use FileOp on a .git path (High risk) as a proxy — same engine,
    //    same Auto mode gate, same ApprovalRequired variant.
    let high_risk_request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: ctx.agent_id.clone(),
        path: "/repo/.git/config".to_string(),
        op: "read".to_string(),
    });
    let response = engine.evaluate(high_risk_request, None);
    assert!(
        matches!(response, PermissionResponse::ApprovalRequired { .. }),
        "Auto mode + high-risk FileOp should return ApprovalRequired, got: {:?}",
        response
    );

    // 2. Route the ApprovalRequired through the approval flow — the same
    //    mechanism the kill tool uses.
    let (_operation_desc, risk_level) = match &response {
        PermissionResponse::ApprovalRequired {
            operation_desc,
            risk_level,
            ..
        } => (operation_desc.clone(), *risk_level),
        _ => unreachable!(),
    };
    let caller = Caller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::InterAgentMsg {
        from: ctx.agent_id.clone(),
        to: "child-agent".to_string(),
    };
    let session_id = ctx.session_id.as_deref().unwrap_or("");
    let mut flow_guard = flow.lock().await;
    let request_id = flow_guard
        .submit_denial(&caller, &body, risk_level, session_id, false)
        .expect("approval flow should accept the submission");

    // 3. Verify the approval flow accepted the request.
    assert!(
        !request_id.is_empty(),
        "approval flow should return a non-empty request_id"
    );
    drop(flow_guard);

    // 4. Now verify the kill tool also correctly routes a Denied response
    //    through the approval flow (the deny_all_engine path).
    let mgr = make_session_manager();
    setup_sessions(&mgr, "parent-deny-route", "child-deny-route").await;
    let tool = SessionsKillTool::new(mgr, deny_all_engine(), make_approval_flow());
    let ctx2 = make_ctx("parent-deny-route");
    let result = tool
        .call(json!({"sessionId": "child-deny-route"}), &ctx2)
        .await;
    let output = result.expect("deny+approval should return Ok");
    assert_eq!(
        output.data["status"], "approval_pending",
        "denied kill should route to approval_pending"
    );
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id from approval flow"
    );
}
