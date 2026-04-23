//! Permission skill - allows agents to query their own permissions
use crate::permission::PermissionResponse;
use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use std::sync::Arc;

pub struct PermissionSkill {
    engine: Option<Arc<crate::permission::PermissionEngine>>,
}

impl PermissionSkill {
    pub fn new() -> Self {
        Self { engine: None }
    }

    pub fn with_engine(engine: Arc<crate::permission::PermissionEngine>) -> Self {
        Self {
            engine: Some(engine),
        }
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
                    let response = engine.check(agent_id, action);
                    match response {
                        PermissionResponse::Allowed { token: _ } => Ok(serde_json::json!({
                            "allowed": true,
                            "agent_id": agent_id,
                            "action": action,
                        })),
                        PermissionResponse::Denied { reason, rule: _ } => Ok(serde_json::json!({
                            "allowed": false,
                            "agent_id": agent_id,
                            "action": action,
                            "reason": reason,
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
    use crate::permission::engine::engine_types::{
        Action, Defaults, Effect, Rule, RuleSet, Subject,
    };
    use std::collections::HashMap;

    fn make_engine_with_allow_rule() -> Arc<crate::permission::PermissionEngine> {
        use crate::permission::engine::engine_types::MatchType;
        let rules = RuleSet {
            version: "1".to_string(),
            rules: vec![Rule {
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
            }],
            defaults: Defaults::default(),
            template_includes: vec![],
            agent_creators: HashMap::new(),
        };
        Arc::new(crate::permission::PermissionEngine::new(rules))
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
