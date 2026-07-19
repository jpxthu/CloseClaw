//! Permission skill - allows agents to query their own permissions
use crate::registry::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use closeclaw_config::agents::{AgentPermissionProvider, NoopPermissionProvider};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::engine::engine_types::{PermissionRequest, PermissionRequestBody};
use closeclaw_permission::PermissionResponse;
use std::sync::Arc;

pub struct PermissionSkill {
    engine: Option<Arc<tokio::sync::RwLock<closeclaw_permission::PermissionEngine>>>,
    session_manager: Option<Arc<SessionManager>>,
    agent_permissions: Arc<dyn AgentPermissionProvider + Send + Sync>,
}

impl PermissionSkill {
    pub fn new() -> Self {
        Self {
            engine: None,
            session_manager: None,
            agent_permissions: Arc::new(NoopPermissionProvider),
        }
    }

    pub fn with_engine(
        engine: Arc<tokio::sync::RwLock<closeclaw_permission::PermissionEngine>>,
    ) -> Self {
        Self {
            engine: Some(engine),
            session_manager: None,
            agent_permissions: Arc::new(NoopPermissionProvider),
        }
    }

    pub fn with_session_manager(mut self, session_manager: Arc<SessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    pub fn with_agent_permissions(
        mut self,
        agent_permissions: Arc<dyn AgentPermissionProvider + Send + Sync>,
    ) -> Self {
        self.agent_permissions = agent_permissions;
        self
    }
}

impl Default for PermissionSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for PermissionSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "permission_query".to_string(),
            version: "1.0.0".to_string(),
            description: "Query the current agent's permission configuration. ".to_string()
                + "Supported actions: exec, file_read, file_write, network, spawn, tool_call, config_write",
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["query", "list_actions"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "query" => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("agent_id required".to_string()))?;
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("action required".to_string()))?;

                if let Some(ref engine) = self.engine {
                    let body = match action {
                        "command" => PermissionRequestBody::CommandExec {
                            agent: agent_id.to_string(),
                            cmd: "*".to_string(),
                            args: Vec::new(),
                        },
                        "file_read" => PermissionRequestBody::FileOp {
                            agent: agent_id.to_string(),
                            path: "*".to_string(),
                            op: "read".to_string(),
                        },
                        "file_write" => PermissionRequestBody::FileOp {
                            agent: agent_id.to_string(),
                            path: "*".to_string(),
                            op: "write".to_string(),
                        },
                        "network" => PermissionRequestBody::NetOp {
                            agent: agent_id.to_string(),
                            host: "*".to_string(),
                            port: 0,
                        },
                        "spawn" => PermissionRequestBody::InterAgentMsg {
                            from: agent_id.to_string(),
                            to: "*".to_string(),
                        },
                        "tool_call" => PermissionRequestBody::ToolCall {
                            agent: agent_id.to_string(),
                            skill: "*".to_string(),
                            method: "*".to_string(),
                        },
                        "config_write" => PermissionRequestBody::ConfigWrite {
                            agent: agent_id.to_string(),
                            config_file: "*".to_string(),
                        },
                        // ask_user_question is a special ToolCall that the
                        // permission engine may annotate with a context_modifier
                        // in Plan Mode (clarification-only marker).
                        "ask_user_question" => PermissionRequestBody::ToolCall {
                            agent: agent_id.to_string(),
                            skill: "ask_user_question".to_string(),
                            method: "call".to_string(),
                        },
                        _ => PermissionRequestBody::ToolCall {
                            agent: agent_id.to_string(),
                            skill: action.to_string(),
                            method: "unknown".to_string(),
                        },
                    };
                    let request = PermissionRequest::Bare(body);
                    let response = if let (Some(ref sm), Some(sid)) = (
                        &self.session_manager,
                        args.get("session_id").and_then(|v| v.as_str()),
                    ) {
                        let guard = engine.read().await;
                        guard
                            .evaluate_with_chain(
                                request,
                                sm.as_ref(),
                                sid,
                                self.agent_permissions.as_ref(),
                            )
                            .await
                    } else {
                        engine.read().await.evaluate(request, None)
                    };
                    match response {
                        PermissionResponse::Allowed {
                            token: _,
                            context_modifier,
                        } => {
                            let mut result = serde_json::json!({
                                "allowed": true,
                                "agent_id": agent_id,
                                "action": action,
                            });
                            if let Some(modifier) = context_modifier {
                                result["context_modifier"] = serde_json::Value::String(modifier);
                            }
                            Ok(result)
                        }
                        PermissionResponse::Denied {
                            reason,
                            rule: _,
                            risk_level,
                        } => Ok(serde_json::json!({
                            "allowed": false,
                            "agent_id": agent_id,
                            "action": action,
                            "reason": reason,
                            "risk_level": risk_level,
                        })),
                    }
                } else {
                    Ok(serde_json::json!({
                        "allowed": null,
                        "agent_id": agent_id,
                        "action": action,
                        "reason": "permission engine not available",
                    }))
                }
            }
            "list_actions" => Ok(serde_json::json!({
                "actions": [
                    "command",
                    "file_read",
                    "file_write",
                    "network",
                    "spawn",
                    "tool_call",
                    "config_write",
                ]
            })),
            _ => Err(SkillError::MethodNotFound {
                skill: "permission_query".to_string(),
                method: method.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_permission::engine::engine_types::{
        Action, Defaults, Effect, MatchType, Rule, RuleSet, Subject,
    };
    use std::collections::HashMap;

    fn make_engine_with_allow_rule() -> Arc<closeclaw_permission::PermissionEngine> {
        let rules = RuleSet {
            rules: vec![
                Rule {
                    name: "test-allow".to_string(),
                    subject: Subject::AgentOnly {
                        agent: "agent-1".to_string(),
                        match_type: MatchType::Exact,
                    },
                    effect: Effect::Allow,
                    actions: vec![Action::File {
                        operation: "read".to_string(),
                        paths: vec!["*".to_string()],
                    }],
                    template: None,
                    priority: 0,
                },
                Rule {
                    name: "test-user-allow".to_string(),
                    subject: Subject::UserAndAgent {
                        user_id: "*".to_string(),
                        agent: "agent-1".to_string(),
                        user_match: MatchType::Glob,
                        agent_match: MatchType::Exact,
                    },
                    effect: Effect::Allow,
                    actions: vec![Action::File {
                        operation: "read".to_string(),
                        paths: vec!["*".to_string()],
                    }],
                    template: None,
                    priority: 0,
                },
            ],
            defaults: Defaults::default(),
            template_includes: vec![],
            agent_creators: HashMap::new(),
        };
        Arc::new(closeclaw_permission::PermissionEngine::new_with_default_data_root(rules))
    }

    #[test]
    fn test_manifest() {
        let skill = PermissionSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "permission_query");
        assert_eq!(m.version, "1.0.0");
    }

    #[test]
    fn test_methods() {
        let skill = PermissionSkill::new();
        assert_eq!(skill.methods(), vec!["query", "list_actions"]);
    }

    #[test]
    fn test_default() {
        let skill = PermissionSkill::default();
        assert_eq!(skill.manifest().name, "permission_query");
    }

    #[tokio::test]
    async fn test_query_no_engine_returns_null() {
        let skill = PermissionSkill::new();
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "file_read"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_query_with_engine_allowed() {
        let engine = make_engine_with_allow_rule();
        let skill = PermissionSkill::with_engine(engine);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "agent-1", "action": "file_read"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
    }

    #[tokio::test]
    async fn test_query_with_engine_denied() {
        let engine = make_engine_with_allow_rule();
        let skill = PermissionSkill::with_engine(engine);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "unknown-agent", "action": "file_read"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], false);
    }

    #[tokio::test]
    async fn test_query_missing_agent_id() {
        let skill = PermissionSkill::new();
        let result = skill
            .execute("query", serde_json::json!({"action": "file_read"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_query_missing_action() {
        let skill = PermissionSkill::new();
        let result = skill
            .execute("query", serde_json::json!({"agent_id": "a1"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_actions() {
        let skill = PermissionSkill::new();
        let result = skill.execute("list_actions", serde_json::json!({})).await;
        assert!(result.is_ok());
        let binding = result.unwrap();
        let actions = binding["actions"].as_array().unwrap();
        assert!(actions.len() >= 7);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let skill = PermissionSkill::new();
        let result = skill.execute("nonexistent", serde_json::json!({})).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------
    // context_modifier propagation tests
    // -----------------------------------------------------------------

    use closeclaw_common::session_mode::SessionMode;
    use closeclaw_common::session_mode_query::SessionModeQuery;
    use closeclaw_permission::engine::engine_eval::PermissionEngine;

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

    fn make_plan_mode_engine(agent_id: &str) -> Arc<PermissionEngine> {
        use closeclaw_permission::rules::RuleSetBuilder;

        let query = Arc::new(MockModeQuery::new().with_mode(agent_id, SessionMode::Plan));
        let ruleset = RuleSetBuilder::new()
            .default_file_read(Effect::Allow)
            .default_file_write(Effect::Allow)
            .default_command(Effect::Allow)
            .default_network(Effect::Allow)
            .default_inter_agent(Effect::Allow)
            .default_config(Effect::Allow)
            .default_tool_call(Effect::Allow)
            .build()
            .unwrap();
        Arc::new(
            PermissionEngine::new_with_default_data_root(ruleset).with_session_mode_query(query),
        )
    }

    fn make_normal_mode_engine(agent_id: &str) -> Arc<PermissionEngine> {
        use closeclaw_permission::rules::RuleSetBuilder;

        let query = Arc::new(MockModeQuery::new().with_mode(agent_id, SessionMode::Normal));
        let ruleset = RuleSetBuilder::new()
            .default_file_read(Effect::Allow)
            .default_file_write(Effect::Allow)
            .default_command(Effect::Allow)
            .default_network(Effect::Allow)
            .default_inter_agent(Effect::Allow)
            .default_config(Effect::Allow)
            .default_tool_call(Effect::Allow)
            .build()
            .unwrap();
        Arc::new(
            PermissionEngine::new_with_default_data_root(ruleset).with_session_mode_query(query),
        )
    }

    #[tokio::test]
    async fn test_plan_mode_ask_user_question_returns_context_modifier() {
        let engine = make_plan_mode_engine("agent-pm");
        let skill = PermissionSkill::with_engine(engine);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "agent-pm", "action": "ask_user_question"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
        assert!(
            v.get("context_modifier").is_some(),
            "plan mode ask_user_question should include context_modifier"
        );
        let cm = v["context_modifier"].as_str().unwrap();
        assert!(
            cm.contains("clarification only"),
            "context_modifier should mention clarification only, got: {}",
            cm
        );
    }

    #[tokio::test]
    async fn test_normal_mode_ask_user_question_no_context_modifier() {
        let engine = make_normal_mode_engine("agent-nm");
        let skill = PermissionSkill::with_engine(engine);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "agent-nm", "action": "ask_user_question"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
        assert!(
            v.get("context_modifier").is_none(),
            "normal mode ask_user_question should NOT include context_modifier"
        );
    }

    #[tokio::test]
    async fn test_plan_mode_other_action_no_context_modifier() {
        let engine = make_plan_mode_engine("agent-pm2");
        let skill = PermissionSkill::with_engine(engine);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "agent-pm2", "action": "file_read"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
        assert!(
            v.get("context_modifier").is_none(),
            "plan mode file_read should NOT include context_modifier"
        );
    }
}
