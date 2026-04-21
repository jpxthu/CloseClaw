//! Skill discovery skill - allows agents to search and install skills from ClawHub
use crate::permission::PermissionResponse;
use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use std::sync::Arc;

#[allow(clippy::new_without_default)]
pub struct SkillDiscoverySkill {
    engine: Option<Arc<crate::permission::PermissionEngine>>,
}

impl Default for SkillDiscoverySkill {
    fn default() -> Self {
        Self { engine: None }
    }
}

impl SkillDiscoverySkill {
    pub fn new() -> Self {
        Self { engine: None }
    }

    pub fn with_engine(engine: Arc<crate::permission::PermissionEngine>) -> Self {
        Self {
            engine: Some(engine),
        }
    }
}

#[async_trait]
impl Skill for SkillDiscoverySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_discovery".to_string(),
            version: "1.0.0".to_string(),
            description: "Search, install, and manage skills from ClawHub marketplace. Use find to search, install to add, list to see installed, update to upgrade.".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec!["clawhub".to_string()],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["find", "install", "list", "update"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "find" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("query required".to_string()))?;
                let output = tokio::process::Command::new("clawhub")
                    .args(["search", query])
                    .output()
                    .await
                    .map_err(|e| {
                        SkillError::ExecutionFailed(format!("clawhub search failed: {}", e))
                    })?;
                Ok(
                    serde_json::json!({"query": query, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "install" => {
                let agent_id = args
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("agent_id required".to_string()))?;
                let skill = args
                    .get("skill")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("skill name required".to_string()))?;
                let version = args.get("version").and_then(|v| v.as_str());

                if let Some(ref engine) = self.engine {
                    match engine.check(agent_id, "spawn") {
                        PermissionResponse::Allowed { .. } => {}
                        PermissionResponse::Denied { reason, .. } => {
                            return Err(SkillError::PermissionDenied(reason));
                        }
                    }
                }

                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["install", skill]);
                if let Some(v) = version {
                    cmd.arg("--version").arg(v);
                }
                let output = cmd.output().await.map_err(|e| {
                    SkillError::ExecutionFailed(format!("clawhub install failed: {}", e))
                })?;
                Ok(
                    serde_json::json!({"skill": skill, "version": version, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "list" => {
                let output = tokio::process::Command::new("clawhub")
                    .args(["list"])
                    .output()
                    .await
                    .map_err(|e| {
                        SkillError::ExecutionFailed(format!("clawhub list failed: {}", e))
                    })?;
                Ok(
                    serde_json::json!({"output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            "update" => {
                let skill = args.get("skill").and_then(|v| v.as_str());
                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["update"]);
                if let Some(s) = skill {
                    cmd.arg(s);
                } else {
                    cmd.arg("--all");
                }
                let output = cmd.output().await.map_err(|e| {
                    SkillError::ExecutionFailed(format!("clawhub update failed: {}", e))
                })?;
                Ok(
                    serde_json::json!({"skill": skill, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}),
                )
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "skill_discovery".to_string(),
                method: method.to_string(),
            }),
        }
    }
}
