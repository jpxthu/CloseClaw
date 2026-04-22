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
