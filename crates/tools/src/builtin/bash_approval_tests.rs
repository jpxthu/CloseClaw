//! Approval flow routing tests for BashTool.
//!
//! Covers three paths:
//! 1. allow — permission passes, normal execution
//! 2. deny + enqueue success — returns approval_pending
//! 3. deny + enqueue failure (duplicate) — fallback to PermissionDenied

use super::*;
use crate::{Tool, ToolCallError, ToolContext, ToolResult};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_risk::RiskLevel;
use closeclaw_permission::engine::engine_types::{
    Action, Caller, Effect, MatchType, PermissionRequestBody, Rule, Subject,
};
use closeclaw_permission::rules::RuleSetBuilder;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn deny_all_engine() -> Arc<PermissionEngine> {
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new().build().unwrap(),
    ))
}

fn allow_all_engine() -> Arc<PermissionEngine> {
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
    Arc::new(PermissionEngine::new_with_default_data_root(
        RuleSetBuilder::new()
            .rule(agent_rule)
            .rule(user_rule)
            .build()
            .unwrap(),
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
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
    )))
}

fn make_tool(perm: Arc<PermissionEngine>) -> BashTool {
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
