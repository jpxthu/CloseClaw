//! Skill permission wrapper implementations.
//!
//! Provides [`SkillPermissionEngineWrapper`] and [`SkillApprovalFlowWrapper`]
//! that implement the trait abstractions defined in
//! [`closeclaw_common::permission_types`], allowing `closeclaw-skills` to
//! depend on traits rather than concrete permission types.

use crate::approval_flow::ApprovalFlow;
use crate::engine::engine_eval::PermissionEngine;
use crate::engine::engine_risk::RiskLevel as EngineRiskLevel;
use crate::engine::engine_types::{Caller, PermissionRequest, PermissionRequestBody};
use async_trait::async_trait;
use closeclaw_common::permission_types::{
    CallerInfo, PermissionEvalResult, RiskLevel, SkillApprovalSubmitter, SkillPermissionChecker,
};
use std::sync::Arc;

/// Wrapper around [`PermissionEngine`] implementing [`SkillPermissionChecker`].
///
/// Maps skill-level `action`/`resource`/`details` to engine requests and
/// converts [`PermissionResponse`](crate::PermissionResponse) to
/// [`PermissionEvalResult`].
pub struct SkillPermissionEngineWrapper {
    engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
}

impl SkillPermissionEngineWrapper {
    /// Create a new wrapper.
    pub fn new(engine: Arc<tokio::sync::RwLock<PermissionEngine>>) -> Self {
        Self { engine }
    }

    /// Get a reference to the inner engine.
    pub fn engine(&self) -> &Arc<tokio::sync::RwLock<PermissionEngine>> {
        &self.engine
    }
}

/// Type alias for a shared skill permission engine wrapper.
pub type SharedSkillPermissionEngineWrapper = Arc<SkillPermissionEngineWrapper>;

#[async_trait]
impl SkillPermissionChecker for SkillPermissionEngineWrapper {
    async fn check_permission(
        &self,
        action: &str,
        resource: &str,
        details: serde_json::Value,
    ) -> PermissionEvalResult {
        let agent_id = details
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let request = build_request(action, resource, &details, &agent_id);
        let extra_deny = self
            .engine
            .read()
            .await
            .get_agent_deny_subjects(&agent_id, &agent_id);
        let response = self.engine.read().await.evaluate(request, Some(extra_deny));

        to_eval_result(response)
    }
}

/// Wrapper around [`ApprovalFlow`] implementing [`SkillApprovalSubmitter`].
///
/// Converts skill-level denial information into the engine's
/// [`PermissionRequestBody`] and [`Caller`] and delegates to
/// [`ApprovalFlow::submit_denial`].
pub struct SkillApprovalFlowWrapper {
    flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

impl SkillApprovalFlowWrapper {
    /// Create a new wrapper.
    pub fn new(flow: Arc<tokio::sync::Mutex<ApprovalFlow>>) -> Self {
        Self { flow }
    }

    /// Get a reference to the inner approval flow.
    pub fn flow(&self) -> &Arc<tokio::sync::Mutex<ApprovalFlow>> {
        &self.flow
    }
}

/// Type alias for a shared skill approval flow wrapper.
pub type SharedSkillApprovalFlowWrapper = Arc<SkillApprovalFlowWrapper>;

#[async_trait]
impl SkillApprovalSubmitter for SkillApprovalFlowWrapper {
    async fn submit_denial(
        &self,
        action: &str,
        resource: &str,
        reason: &str,
        risk_level: RiskLevel,
        session_id: &str,
        caller_info: &CallerInfo,
    ) -> Option<String> {
        let engine_risk = to_engine_risk_level(risk_level);
        let body = build_denial_body(action, resource, reason);
        let caller = Caller {
            user_id: caller_info.user_id.clone(),
            agent: caller_info.agent.clone(),
            creator_id: caller_info.creator_id.clone(),
        };
        let mut flow = self.flow.lock().await;
        flow.submit_denial(&caller, &body, engine_risk, session_id, false)
    }
}

// ── Internal helpers ────────────────────────────────────────────────────

/// Convert engine risk level to the common risk level type.
fn to_common_risk_level(level: EngineRiskLevel) -> RiskLevel {
    match level {
        EngineRiskLevel::Low => RiskLevel::Low,
        EngineRiskLevel::Medium => RiskLevel::Medium,
        EngineRiskLevel::High => RiskLevel::High,
        EngineRiskLevel::Critical => RiskLevel::Critical,
    }
}

/// Convert common risk level to the engine risk level type.
fn to_engine_risk_level(level: RiskLevel) -> EngineRiskLevel {
    match level {
        RiskLevel::Low => EngineRiskLevel::Low,
        RiskLevel::Medium => EngineRiskLevel::Medium,
        RiskLevel::High => EngineRiskLevel::High,
        RiskLevel::Critical => EngineRiskLevel::Critical,
    }
}

/// Convert a [`PermissionResponse`](crate::PermissionResponse) to
/// [`PermissionEvalResult`].
fn to_eval_result(response: crate::PermissionResponse) -> PermissionEvalResult {
    match response {
        crate::PermissionResponse::Allowed {
            context_modifier, ..
        } => PermissionEvalResult::Allowed { context_modifier },
        crate::PermissionResponse::Denied {
            reason, risk_level, ..
        } => PermissionEvalResult::Denied {
            reason,
            risk_level: to_common_risk_level(risk_level),
        },
    }
}

/// Build a [`PermissionRequest`] from skill action, resource, and details.
fn build_request(
    action: &str,
    resource: &str,
    details: &serde_json::Value,
    agent_id: &str,
) -> PermissionRequest {
    let body = build_request_body(action, resource, details, agent_id);
    PermissionRequest::Bare(body)
}

/// Build a [`PermissionRequestBody`] from skill action, resource, and details.
fn build_request_body(
    action: &str,
    resource: &str,
    details: &serde_json::Value,
    agent_id: &str,
) -> PermissionRequestBody {
    build_body_for_action(action, resource, details, agent_id)
}

/// Build a [`PermissionRequestBody`] for denial submission.
fn build_denial_body(action: &str, resource: &str, _reason: &str) -> PermissionRequestBody {
    build_body_for_action(action, resource, &serde_json::json!({}), "")
}

/// Common body builder shared by request and denial construction.
fn build_body_for_action(
    action: &str,
    resource: &str,
    details: &serde_json::Value,
    agent_id: &str,
) -> PermissionRequestBody {
    match action {
        "file_read" => PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: resource.to_string(),
            op: "read".to_string(),
        },
        "file_write" => PermissionRequestBody::FileOp {
            agent: agent_id.to_string(),
            path: resource.to_string(),
            op: "write".to_string(),
        },
        "command" => PermissionRequestBody::CommandExec {
            agent: agent_id.to_string(),
            cmd: resource.to_string(),
            args: details
                .get("args")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        },
        "network" => {
            let host = details
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or(resource)
                .to_string();
            let port = details.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            PermissionRequestBody::NetOp {
                agent: agent_id.to_string(),
                host,
                port,
            }
        }
        "spawn" => PermissionRequestBody::InterAgentMsg {
            from: agent_id.to_string(),
            to: resource.to_string(),
        },
        "tool_call" | "ask_user_question" => {
            let skill = details
                .get("skill")
                .and_then(|v| v.as_str())
                .unwrap_or(action)
                .to_string();
            let method = details
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("call")
                .to_string();
            PermissionRequestBody::ToolCall {
                agent: agent_id.to_string(),
                skill,
                method,
            }
        }
        "config_write" => PermissionRequestBody::ConfigWrite {
            agent: agent_id.to_string(),
            config_file: resource.to_string(),
        },
        "message" => {
            let target = details
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or(resource)
                .to_string();
            PermissionRequestBody::MessageSend {
                agent: agent_id.to_string(),
                direction: Default::default(),
                target,
            }
        }
        _ => PermissionRequestBody::ToolCall {
            agent: agent_id.to_string(),
            skill: action.to_string(),
            method: resource.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::engine_types::{Defaults, Effect, MatchType, Rule, RuleSet, Subject};
    use std::collections::HashMap;

    fn make_engine_with_rules(rules: Vec<Rule>) -> Arc<tokio::sync::RwLock<PermissionEngine>> {
        let ruleset = RuleSet {
            rules,
            defaults: Defaults::default(),
            template_includes: vec![],
            agent_creators: HashMap::new(),
            ..Default::default()
        };
        Arc::new(tokio::sync::RwLock::new(
            PermissionEngine::new_with_default_data_root(ruleset),
        ))
    }

    #[test]
    fn test_to_common_risk_level() {
        assert_eq!(to_common_risk_level(EngineRiskLevel::Low), RiskLevel::Low);
        assert_eq!(
            to_common_risk_level(EngineRiskLevel::Medium),
            RiskLevel::Medium
        );
        assert_eq!(to_common_risk_level(EngineRiskLevel::High), RiskLevel::High);
        assert_eq!(
            to_common_risk_level(EngineRiskLevel::Critical),
            RiskLevel::Critical
        );
    }

    #[test]
    fn test_to_engine_risk_level() {
        assert_eq!(to_engine_risk_level(RiskLevel::Low), EngineRiskLevel::Low);
        assert_eq!(
            to_engine_risk_level(RiskLevel::Medium),
            EngineRiskLevel::Medium
        );
        assert_eq!(to_engine_risk_level(RiskLevel::High), EngineRiskLevel::High);
        assert_eq!(
            to_engine_risk_level(RiskLevel::Critical),
            EngineRiskLevel::Critical
        );
    }

    #[test]
    fn test_to_eval_result_allowed() {
        let resp = crate::PermissionResponse::Allowed {
            token: "tok".to_string(),
            context_modifier: None,
        };
        match to_eval_result(resp) {
            PermissionEvalResult::Allowed { context_modifier } => {
                assert!(context_modifier.is_none());
            }
            other => panic!("expected Allowed, got {:?}", other),
        }
    }

    #[test]
    fn test_to_eval_result_allowed_with_context_modifier() {
        let resp = crate::PermissionResponse::Allowed {
            token: "tok".to_string(),
            context_modifier: Some("clarification_only".to_string()),
        };
        match to_eval_result(resp) {
            PermissionEvalResult::Allowed { context_modifier } => {
                assert_eq!(context_modifier.as_deref(), Some("clarification_only"));
            }
            other => panic!("expected Allowed, got {:?}", other),
        }
    }

    #[test]
    fn test_to_eval_result_denied() {
        let resp = crate::PermissionResponse::Denied {
            reason: "no".to_string(),
            rule: "r".to_string(),
            risk_level: EngineRiskLevel::High,
        };
        match to_eval_result(resp) {
            PermissionEvalResult::Denied { reason, risk_level } => {
                assert_eq!(reason, "no");
                assert_eq!(risk_level, RiskLevel::High);
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_file_read() {
        let body = build_request_body(
            "file_read",
            "/tmp/test.txt",
            &serde_json::json!({}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::FileOp { agent, path, op } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(path, "/tmp/test.txt");
                assert_eq!(op, "read");
            }
            other => panic!("expected FileOp, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_command() {
        let body = build_request_body(
            "command",
            "ls",
            &serde_json::json!({"args": ["-la", "/tmp"]}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::CommandExec { agent, cmd, args } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(cmd, "ls");
                assert_eq!(args, vec!["-la".to_string(), "/tmp".to_string()]);
            }
            other => panic!("expected CommandExec, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_network() {
        let body = build_request_body(
            "network",
            "example.com",
            &serde_json::json!({"host": "example.com", "port": 443}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::NetOp { agent, host, port } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(host, "example.com");
                assert_eq!(port, 443);
            }
            other => panic!("expected NetOp, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_tool_call() {
        let body = build_request_body(
            "tool_call",
            "heartbeat",
            &serde_json::json!({"skill": "heartbeat", "method": "ping"}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::ToolCall {
                agent,
                skill,
                method,
            } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(skill, "heartbeat");
                assert_eq!(method, "ping");
            }
            other => panic!("expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_ask_user_question() {
        let body = build_request_body(
            "ask_user_question",
            "resource",
            &serde_json::json!({"skill": "ask_user_question"}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::ToolCall {
                agent,
                skill,
                method,
            } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(skill, "ask_user_question");
                assert_eq!(method, "call");
            }
            other => panic!("expected ToolCall for ask_user_question, got {:?}", other),
        }
    }

    #[test]
    fn test_build_denial_body_ask_user_question() {
        let body = build_denial_body("ask_user_question", "resource", "denied");
        match body {
            PermissionRequestBody::ToolCall { skill, method, .. } => {
                assert_eq!(skill, "ask_user_question");
                assert_eq!(method, "call");
            }
            other => panic!("expected ToolCall for ask_user_question, got {:?}", other),
        }
    }

    #[test]
    fn test_build_request_body_fallback() {
        let body = build_request_body(
            "unknown_action",
            "resource",
            &serde_json::json!({}),
            "agent-1",
        );
        match body {
            PermissionRequestBody::ToolCall {
                agent,
                skill,
                method,
            } => {
                assert_eq!(agent, "agent-1");
                assert_eq!(skill, "unknown_action");
                assert_eq!(method, "resource");
            }
            other => panic!("expected ToolCall fallback, got {:?}", other),
        }
    }

    #[test]
    fn test_build_denial_body_file_read() {
        let body = build_denial_body("file_read", "/tmp/test.txt", "denied");
        match body {
            PermissionRequestBody::FileOp { path, op, .. } => {
                assert_eq!(path, "/tmp/test.txt");
                assert_eq!(op, "read");
            }
            other => panic!("expected FileOp, got {:?}", other),
        }
    }

    #[test]
    fn test_build_denial_body_command() {
        let body = build_denial_body("command", "rm -rf /", "dangerous");
        match body {
            PermissionRequestBody::CommandExec { cmd, .. } => {
                assert_eq!(cmd, "rm -rf /");
            }
            other => panic!("expected CommandExec, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_wrapper_allowed() {
        let engine = make_engine_with_rules(vec![Rule {
            name: "allow-all".to_string(),
            subject: Subject::AgentOnly {
                agent: "test-agent".to_string(),
                match_type: MatchType::Exact,
            },
            effect: Effect::Allow,
            actions: vec![crate::engine::engine_types::Action::File {
                operation: "read".to_string(),
                paths: vec!["*".to_string()],
            }],
            template: None,
            priority: 0,
        }]);
        let wrapper = SkillPermissionEngineWrapper::new(engine);
        let details = serde_json::json!({
            "agent_id": "test-agent",
            "user_id": "user-1"
        });
        let result = wrapper
            .check_permission("file_read", "/tmp/test.txt", details)
            .await;
        match result {
            PermissionEvalResult::Allowed { .. } => {}
            other => panic!("expected Allowed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_wrapper_denied() {
        let engine = make_engine_with_rules(vec![]);
        let wrapper = SkillPermissionEngineWrapper::new(engine);
        let details = serde_json::json!({
            "agent_id": "test-agent",
            "user_id": "user-1"
        });
        let result = wrapper
            .check_permission("file_read", "/tmp/test.txt", details)
            .await;
        match result {
            PermissionEvalResult::Denied { reason, risk_level } => {
                assert!(!reason.is_empty());
                assert_eq!(risk_level, RiskLevel::Low);
            }
            other => panic!("expected Denied, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_wrapper_no_agent_id_defaults() {
        let engine = make_engine_with_rules(vec![]);
        let wrapper = SkillPermissionEngineWrapper::new(engine);
        let details = serde_json::json!({});
        let result = wrapper
            .check_permission("file_read", "/tmp/test.txt", details)
            .await;
        // Should not panic — agent_id defaults to "unknown"
        assert!(matches!(result, PermissionEvalResult::Denied { .. }));
    }
}
