//! File operations skill
use crate::permission::PermissionResponse;
use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;
use std::sync::Arc;

#[allow(clippy::new_without_default)]
pub struct FileOpsSkill {
    engine: Option<Arc<crate::permission::PermissionEngine>>,
}

impl Default for FileOpsSkill {
    fn default() -> Self {
        Self { engine: None }
    }
}

impl FileOpsSkill {
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
impl Skill for FileOpsSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "file_ops".to_string(),
            version: "1.0.0".to_string(),
            description: "File system operations: read, write, list, delete".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["read", "write", "list", "delete", "exists"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        let action = match method {
            "read" | "exists" | "list" => "file_read",
            "write" | "delete" => "file_write",
            _ => {
                return Err(SkillError::MethodNotFound {
                    skill: "file_ops".to_string(),
                    method: method.to_string(),
                });
            }
        };

        if let Some(ref engine) = self.engine {
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SkillError::InvalidArgs("agent_id required".to_string()))?;
            match engine.check(agent_id, action) {
                PermissionResponse::Allowed { .. } => {}
                PermissionResponse::Denied { reason, .. } => {
                    return Err(SkillError::PermissionDenied(reason));
                }
            }
        }

        match method {
            "read" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                std::fs::read_to_string(path)
                    .map(|content| serde_json::json!({ "content": content }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "write" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("content required".to_string()))?;
                std::fs::write(path, content)
                    .map(|_| serde_json::json!({ "success": true }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "exists" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                Ok(serde_json::json!({ "exists": std::path::Path::new(path).exists() }))
            }
            "delete" => {
                let path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                std::fs::remove_file(path)
                    .map(|_| serde_json::json!({ "success": true }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "list" => {
                let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let entries: Vec<_> = std::fs::read_dir(path)
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))?
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                Ok(serde_json::json!({ "entries": entries }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "file_ops".to_string(),
                method: method.to_string(),
            }),
        }
    }
}
