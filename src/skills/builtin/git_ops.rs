//! Git operations skill
use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;

#[derive(Default)]
pub struct GitOpsSkill;

impl GitOpsSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for GitOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "git_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "Git operations: status, commit, push, pull".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["status", "commit", "push", "pull", "log"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "status" => {
                let output = std::process::Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?;
                Ok(serde_json::json!({
                    "output": String::from_utf8_lossy(&output.stdout)
                }))
            }
            "log" => {
                let output = std::process::Command::new("git")
                    .args(["log", "--oneline", "-10"])
                    .output()
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?;
                Ok(serde_json::json!({
                    "output": String::from_utf8_lossy(&output.stdout)
                }))
            }
            "commit" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("message required".to_string()))?;
                let output = std::process::Command::new("git")
                    .args(["commit", "-m", message])
                    .output()
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?;
                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "output": String::from_utf8_lossy(&output.stdout),
                    "error": String::from_utf8_lossy(&output.stderr)
                }))
            }
            "push" => {
                let output = std::process::Command::new("git")
                    .args(["push"])
                    .output()
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?;
                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "output": String::from_utf8_lossy(&output.stdout),
                    "error": String::from_utf8_lossy(&output.stderr)
                }))
            }
            "pull" => {
                let output = std::process::Command::new("git")
                    .args(["pull"])
                    .output()
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?;
                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "output": String::from_utf8_lossy(&output.stdout),
                    "error": String::from_utf8_lossy(&output.stderr)
                }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "git_ops".to_string(),
                method: method.to_string(),
            }),
        }
    }
}
