//! Permission skill - allows agents to query their own permissions
use crate::registry::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use closeclaw_config::agents::AgentPermissions;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::engine::engine_types::{PermissionRequest, PermissionRequestBody};
use closeclaw_permission::PermissionResponse;
use std::collections::HashMap;
use std::sync::Arc;

pub struct PermissionSkill {
    engine: Option<Arc<closeclaw_permission::PermissionEngine>>,
    session_manager: Option<Arc<SessionManager>>,
    agent_permissions: HashMap<String, AgentPermissions>,
}

impl PermissionSkill {
    pub fn new() -> Self {
        Self {
            engine: None,
            session_manager: None,
            agent_permissions: HashMap::new(),
        }
    }

    pub fn with_engine(engine: Arc<closeclaw_permission::PermissionEngine>) -> Self {
        Self {
            engine: Some(engine),
            session_manager: None,
            agent_permissions: HashMap::new(),
        }
    }

    pub fn with_session_manager(mut self, session_manager: Arc<SessionManager>) -> Self {
        self.session_manager = Some(session_manager);
        self
    }

    pub fn with_agent_permissions(
        mut self,
        agent_permissions: HashMap<String, AgentPermissions>,
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
                        "exec" => PermissionRequestBody::CommandExec {
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
                        engine
                            .evaluate_with_chain(request, sm.as_ref(), sid, &self.agent_permissions)
                            .await
                    } else {
                        engine.evaluate(request, None)
                    };
                    match response {
                        PermissionResponse::Allowed { token: _ } => Ok(serde_json::json!({
                            "allowed": true,
                            "agent_id": agent_id,
                            "action": action,
                        })),
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
                    "exec",
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
}
