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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_manifest() {
        let skill = FileOpsSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "file_ops");
        assert_eq!(m.version, "1.0.0");
    }

    #[test]
    fn test_methods() {
        let skill = FileOpsSkill::new();
        assert_eq!(
            skill.methods(),
            vec!["read", "write", "list", "delete", "exists"]
        );
    }

    #[tokio::test]
    async fn test_read_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();

        let skill = FileOpsSkill::new();
        let result = skill
            .execute("read", serde_json::json!({"path": path.to_str().unwrap()}))
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["content"], "hello world");
    }

    #[tokio::test]
    async fn test_read_missing_path() {
        let skill = FileOpsSkill::new();
        let result = skill.execute("read", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("path")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("out.txt");

        let skill = FileOpsSkill::new();
        let result = skill
            .execute(
                "write",
                serde_json::json!({"path": path.to_str().unwrap(), "content": "data"}),
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["success"], true);

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
    async fn test_write_missing_content() {
        let skill = FileOpsSkill::new();
        let result = skill
            .execute("write", serde_json::json!({"path": "/tmp/x"}))
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::InvalidArgs(msg) => assert!(msg.contains("content")),
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_exists_true_and_false() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("real.txt");
        std::fs::write(&path, "").unwrap();

        let skill = FileOpsSkill::new();
        let result = skill
            .execute(
                "exists",
                serde_json::json!({"path": path.to_str().unwrap()}),
            )
            .await;
        assert_eq!(result.unwrap()["exists"], true);

        let fake = dir.path().join("fake.txt");
        let result = skill
            .execute(
                "exists",
                serde_json::json!({"path": fake.to_str().unwrap()}),
            )
            .await;
        assert_eq!(result.unwrap()["exists"], false);
    }

    #[tokio::test]
    async fn test_delete_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("del.txt");
        std::fs::write(&path, "bye").unwrap();

        let skill = FileOpsSkill::new();
        let result = skill
            .execute(
                "delete",
                serde_json::json!({"path": path.to_str().unwrap()}),
            )
            .await;
        assert!(result.is_ok());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_list_dir() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let skill = FileOpsSkill::new();
        let result = skill
            .execute(
                "list",
                serde_json::json!({"path": dir.path().to_str().unwrap()}),
            )
            .await;
        let binding = result.unwrap();
        let entries = binding["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let skill = FileOpsSkill::new();
        let result = skill.execute("nonexistent", serde_json::json!({})).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            SkillError::MethodNotFound { skill, .. } => assert_eq!(skill, "file_ops"),
            other => panic!("expected MethodNotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_default() {
        let skill = FileOpsSkill::default();
        let m = skill.manifest();
        assert_eq!(m.name, "file_ops");
    }
}
