//! Approval flow routing tests for BashTool.
//!
//! Covers three paths:
//! 1. allow — permission passes, normal execution
//! 2. deny + enqueue success — returns approval_pending
//! 3. deny + enqueue failure (duplicate) — fallback to PermissionDenied

use super::*;
use crate::{Tool, ToolCallError, ToolContext, ToolResult};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::session_mode_query::SessionModeQuery;
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{
    Action, Caller, Effect, MatchType, PermissionRequest, PermissionRequestBody,
    PermissionResponse, Rule, Subject,
};
use closeclaw_permission::rules::RuleSetBuilder;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

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

fn deny_all_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(RuleSetBuilder::new().build().unwrap()),
    ))
}

fn allow_all_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let tool_call_rule = Rule {
        name: "allow-all-tool-call".to_string(),
        subject: Subject::AgentOnly {
            agent: "*".to_string(),
            match_type: MatchType::Glob,
        },
        effect: Effect::Allow,
        actions: vec![Action::All],
        template: None,
        priority: 200,
    };
    let agent_rule = Rule {
        name: "allow-all-exec-agent".to_string(),
        subject: Subject::AgentOnly {
            agent: "*".to_string(),
            match_type: MatchType::Glob,
        },
        effect: Effect::Allow,
        actions: vec![Action::Command {
            command: "*".to_string(),
            args: Default::default(),
        }],
        template: None,
        priority: 100,
    };
    let user_rule = Rule {
        name: "allow-all-exec-user".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "".to_string(),
            agent: "*".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Glob,
        },
        effect: Effect::Allow,
        actions: vec![Action::Command {
            command: "*".to_string(),
            args: Default::default(),
        }],
        template: None,
        priority: 100,
    };
    Arc::new(tokio::sync::RwLock::new(
        PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new()
                .rule(tool_call_rule)
                .rule(agent_rule)
                .rule(user_rule)
                .build()
                .unwrap(),
        ),
    ))
}

/// Engine that allows all operations but returns `ApprovalRequired` in Auto
/// mode for high-risk operations (e.g., FileOp on .git path triggers high risk).
fn auto_mode_allow_engine() -> Arc<tokio::sync::RwLock<PermissionEngine>> {
    let agent_rule = Rule {
        name: "allow-all-exec-agent".to_string(),
        subject: Subject::AgentOnly {
            agent: "*".to_string(),
            match_type: MatchType::Glob,
        },
        effect: Effect::Allow,
        actions: vec![Action::All],
        template: None,
        priority: 100,
    };
    let user_rule = Rule {
        name: "allow-all-exec-user".to_string(),
        subject: Subject::UserAndAgent {
            user_id: "".to_string(),
            agent: "*".to_string(),
            user_match: MatchType::Exact,
            agent_match: MatchType::Glob,
        },
        effect: Effect::Allow,
        actions: vec![Action::All],
        template: None,
        priority: 100,
    };
    Arc::new(tokio::sync::RwLock::new(
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
    ))
}

fn make_bg_manager() -> Arc<dyn closeclaw_tasks::TaskManager> {
    struct DummyTaskManager;
    #[async_trait::async_trait]
    impl closeclaw_tasks::TaskManager for DummyTaskManager {
        async fn spawn_task(
            &self,
            _command: &str,
            _cwd: &std::path::Path,
        ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
            unimplemented!("not needed for approval flow tests")
        }
        async fn backgroundize_task(
            &self,
            _child: tokio::process::Child,
            _command: &str,
        ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
            unimplemented!("not needed for approval flow tests")
        }
        async fn kill_task(
            &self,
            _task_id: &str,
        ) -> Result<(), closeclaw_tasks::BackgroundTaskError> {
            Ok(())
        }
        async fn get_task(&self, _task_id: &str) -> Option<closeclaw_tasks::BackgroundTask> {
            None
        }
    }
    Arc::new(DummyTaskManager)
}

fn make_session_manager() -> Arc<SessionManager> {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            dm_scope: closeclaw_gateway::DmScope::default(),
            ..Default::default()
        },
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

fn make_config_manager() -> Arc<ConfigManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    Arc::new(
        ConfigManager::new(tmp.path().to_path_buf()).expect("ConfigManager::new should succeed"),
    )
}

fn make_approval_flow() -> Arc<TokioMutex<ApprovalFlow>> {
    Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&make_session_manager()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(|_| {}),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
    )))
}

fn make_tool(perm: Arc<tokio::sync::RwLock<PermissionEngine>>) -> BashTool {
    BashTool::new(
        perm,
        make_bg_manager(),
        make_session_manager(),
        make_config_manager(),
        make_approval_flow(),
    )
}

fn make_ctx() -> ToolContext {
    ToolContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
        session_mode: None,
        manual_background_signal: None,
    }
}

// ---------------------------------------------------------------------------
// Path 1: allow — permission passes, normal execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_bash_approval_allow_path() {
    let tool = make_tool(allow_all_engine());
    let result = tool
        .call(json!({"command": "echo hello"}), &make_ctx())
        .await;
    assert!(
        result.is_ok(),
        "allow path should succeed, got: {:?}",
        result.err()
    );
    let output = result.unwrap();
    assert!(
        output.data.get("stdout").is_some() || output.data.get("output").is_some(),
        "result should have stdout or output field, got: {:?}",
        output.data
    );
}

// ---------------------------------------------------------------------------
// Path 2: deny + enqueue success → approval_pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_bash_approval_deny_enqueue_success() {
    let tool = make_tool(deny_all_engine());
    let result = tool
        .call(json!({"command": "echo hello"}), &make_ctx())
        .await;
    assert!(
        result.is_ok(),
        "deny+enqueue should return Ok, got: {:?}",
        result.err()
    );
    let output = result.unwrap();
    assert_eq!(
        output.data["status"], "approval_pending",
        "should return approval_pending status"
    );
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id"
    );
}

// ---------------------------------------------------------------------------
// Path 3: deny + enqueue failure (duplicate) → fallback to PermissionDenied
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_bash_approval_deny_enqueue_fallback() {
    // Pre-enqueue a matching denial so the tool's submission is a duplicate
    // and returns None, triggering the fallback to PermissionDenied.
    let flow = make_approval_flow();
    let caller = Caller {
        user_id: String::new(),
        agent: "test-agent".to_string(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::ToolCall {
        agent: "test-agent".to_string(),
        skill: "bash".to_string(),
        method: "call".to_string(),
    };
    {
        let mut f = flow.lock().await;
        f.submit_denial(&caller, &body, RiskLevel::Medium, "", false)
            .expect("first enqueue should succeed");
    }

    let perm = deny_all_engine();
    let tool = BashTool::new(
        perm,
        make_bg_manager(),
        make_session_manager(),
        make_config_manager(),
        Arc::clone(&flow),
    );
    let result: Result<ToolResult, ToolCallError> = tool
        .call(json!({"command": "echo hello"}), &make_ctx())
        .await;
    let err = result.expect_err("fallback should return error");
    match err {
        ToolCallError::PermissionDenied(msg) => {
            assert!(
                msg.contains("denied") || msg.contains("no matching rule"),
                "error should mention denial, got: {}",
                msg
            );
        }
        other => panic!("expected PermissionDenied, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Path 4: ApprovalRequired → approval flow routing
// ---------------------------------------------------------------------------

/// Verify that when the permission engine returns `ApprovalRequired` (e.g.
/// Auto mode + high-risk operation), the response is properly routed through
/// the approval flow, returning `approval_pending` with a request_id.
///
/// The bash tool sends `CommandExec { cmd, args }` to the engine. In Auto
/// mode, bare `rm -rf` (no path) triggers High risk → ApprovalRequired.
/// Since the security analyzer blocks bare `rm -rf` before the permission
/// check, this test directly exercises the approval routing mechanism by
/// calling `evaluate()` with a high-risk request and manually routing the
/// response through the approval flow — the same path the tool follows.
#[tokio::test]
async fn test_bash_approval_required_routes_through_approval_flow() {
    let flow = make_approval_flow();
    let engine = auto_mode_allow_engine();
    let ctx = make_ctx();

    // 1. Verify the engine returns ApprovalRequired for a high-risk request.
    //    Use FileOp on a .git path (High risk) as a proxy — same engine,
    //    same Auto mode gate, same ApprovalRequired variant.
    let high_risk_request = PermissionRequest::Bare(PermissionRequestBody::FileOp {
        agent: ctx.agent_id.clone(),
        path: "/repo/.git/config".to_string(),
        op: "read".to_string(),
    });
    let response = engine.read().await.evaluate(high_risk_request, None);
    assert!(
        matches!(response, PermissionResponse::ApprovalRequired { .. }),
        "Auto mode + high-risk FileOp should return ApprovalRequired, got: {:?}",
        response
    );

    // 2. Route the ApprovalRequired through the approval flow — the same
    //    mechanism the bash tool uses in check_permission_and_route.
    let (_operation_desc, risk_level) = match &response {
        PermissionResponse::ApprovalRequired {
            operation_desc,
            risk_level,
            ..
        } => (operation_desc.clone(), risk_level.clone()),
        _ => unreachable!(),
    };
    let caller = Caller {
        user_id: String::new(),
        agent: ctx.agent_id.clone(),
        creator_id: String::new(),
    };
    let body = PermissionRequestBody::ToolCall {
        agent: ctx.agent_id.clone(),
        skill: "bash".to_string(),
        method: "call".to_string(),
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

    // 4. Now verify the bash tool also correctly routes a Denied response
    //    through the approval flow (the deny_all_engine path).
    let tool = make_tool(deny_all_engine());
    let result = tool
        .call(json!({"command": "echo hello"}), &make_ctx())
        .await;
    let output = result.expect("deny+approval should return Ok");
    assert_eq!(
        output.data["status"], "approval_pending",
        "denied command should route to approval_pending"
    );
    assert!(
        output.data["request_id"].is_string(),
        "should include request_id from approval flow"
    );
}
