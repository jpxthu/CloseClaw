//! Approval flow routing tests for BashTool.
//!
//! Covers two paths:
//! 1. allow — permission passes, normal execution
//! 2. deny + enqueue success — returns approval_pending

use super::*;
use crate::{Tool, ToolContext};
use closeclaw_config::ConfigManager;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_eval::PermissionEngine;
use closeclaw_permission::engine::engine_types::{
    Action, Effect, MatchType, Rule, RuleSet, Subject,
};
use closeclaw_permission::rules::RuleSetBuilder;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

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

fn make_bg_manager() -> Arc<dyn closeclaw_tasks::TaskManager> {
    struct DummyTaskManager;
    #[async_trait::async_trait]
    impl closeclaw_tasks::TaskManager for DummyTaskManager {
        async fn spawn_task(
            &self,
            _command: &str,
            _cwd: &std::path::Path,
            _is_backgrounded: bool,
        ) -> Result<closeclaw_tasks::BackgroundTask, closeclaw_tasks::BackgroundTaskError> {
            unimplemented!("not needed for approval flow tests")
        }
        async fn backgroundize_task(
            &self,
            _child: tokio::process::Child,
            _command: &str,
            _is_backgrounded: bool,
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
        async fn drain_notifications(&self) -> Vec<closeclaw_tasks::CompletionNotification> {
            vec![]
        }
        async fn cleanup_finished(&self) {}
    }
    Arc::new(DummyTaskManager)
}

fn make_session_manager() -> Arc<SessionManager> {
    use closeclaw_gateway::GatewayConfig;
    use closeclaw_session::persistence::ReasoningLevel;
    Arc::new(SessionManager::new(
        &GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 1024,
            ..Default::default()
        },
        None,
        None,
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
        RuleSet::default(),
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
