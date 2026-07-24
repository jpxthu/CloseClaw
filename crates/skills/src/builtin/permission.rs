//! Permission skill - allows agents to query their own permissions
use crate::registry::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use closeclaw_common::permission_types::{PermissionEvalResult, SharedSkillPermissionChecker};

pub struct PermissionSkill {
    engine: Option<SharedSkillPermissionChecker>,
}

impl PermissionSkill {
    pub fn new() -> Self {
        Self { engine: None }
    }

    pub fn with_engine(engine: SharedSkillPermissionChecker) -> Self {
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
                    let resource = "*".to_string();
                    let details = serde_json::json!({
                        "agent_id": agent_id,
                    });
                    match engine.check_permission(action, &resource, details).await {
                        PermissionEvalResult::Allowed { context_modifier } => {
                            let mut resp = serde_json::json!({
                                "allowed": true,
                                "agent_id": agent_id,
                                "action": action,
                            });
                            if let Some(cm) = context_modifier {
                                resp["context_modifier"] = serde_json::json!(cm);
                            }
                            Ok(resp)
                        }
                        PermissionEvalResult::Denied { reason, risk_level } => {
                            let risk_str = match risk_level {
                                closeclaw_common::permission_types::RiskLevel::Low => "low",
                                closeclaw_common::permission_types::RiskLevel::Medium => "medium",
                                closeclaw_common::permission_types::RiskLevel::High => "high",
                                closeclaw_common::permission_types::RiskLevel::Critical => {
                                    "critical"
                                }
                            };
                            Ok(serde_json::json!({
                                "allowed": false,
                                "agent_id": agent_id,
                                "action": action,
                                "reason": reason,
                                "risk_level": risk_str,
                            }))
                        }
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

    // ── Mock checker tests (A-4) ───────────────────────────────────────

    use async_trait::async_trait;
    use closeclaw_common::permission_types::{
        PermissionEvalResult, RiskLevel, SkillPermissionChecker,
    };
    use std::sync::Arc;

    /// A mock permission checker that returns a configurable result.
    struct MockPermissionChecker {
        result: PermissionEvalResult,
    }

    #[async_trait]
    impl SkillPermissionChecker for MockPermissionChecker {
        async fn check_permission(
            &self,
            _action: &str,
            _resource: &str,
            _details: serde_json::Value,
        ) -> PermissionEvalResult {
            self.result.clone()
        }
    }

    #[tokio::test]
    async fn test_query_allowed_without_context_modifier() {
        let checker: Arc<dyn SkillPermissionChecker> = Arc::new(MockPermissionChecker {
            result: PermissionEvalResult::Allowed {
                context_modifier: None,
            },
        });
        let skill = PermissionSkill::with_engine(checker);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "file_read"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
        assert_eq!(v["context_modifier"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn test_query_allowed_with_context_modifier() {
        let checker: Arc<dyn SkillPermissionChecker> = Arc::new(MockPermissionChecker {
            result: PermissionEvalResult::Allowed {
                context_modifier: Some("read_only".to_string()),
            },
        });
        let skill = PermissionSkill::with_engine(checker);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "file_write"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], true);
        assert_eq!(v["context_modifier"], "read_only");
    }

    #[tokio::test]
    async fn test_query_denied_with_risk_level() {
        let checker: Arc<dyn SkillPermissionChecker> = Arc::new(MockPermissionChecker {
            result: PermissionEvalResult::Denied {
                reason: "not permitted".to_string(),
                risk_level: RiskLevel::High,
            },
        });
        let skill = PermissionSkill::with_engine(checker);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "command"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], false);
        assert_eq!(v["reason"], "not permitted");
        assert_eq!(v["risk_level"], "high");
    }

    #[tokio::test]
    async fn test_query_denied_low_risk() {
        let checker: Arc<dyn SkillPermissionChecker> = Arc::new(MockPermissionChecker {
            result: PermissionEvalResult::Denied {
                reason: "blocked".to_string(),
                risk_level: RiskLevel::Low,
            },
        });
        let skill = PermissionSkill::with_engine(checker);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "network"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], false);
        assert_eq!(v["risk_level"], "low");
    }

    #[tokio::test]
    async fn test_query_denied_critical_risk() {
        let checker: Arc<dyn SkillPermissionChecker> = Arc::new(MockPermissionChecker {
            result: PermissionEvalResult::Denied {
                reason: "critical block".to_string(),
                risk_level: RiskLevel::Critical,
            },
        });
        let skill = PermissionSkill::with_engine(checker);
        let result = skill
            .execute(
                "query",
                serde_json::json!({"agent_id": "a1", "action": "spawn"}),
            )
            .await;
        assert!(result.is_ok());
        let v = result.unwrap();
        assert_eq!(v["allowed"], false);
        assert_eq!(v["risk_level"], "critical");
    }
}
