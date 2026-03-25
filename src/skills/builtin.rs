//! Built-in skills - file_ops, git_ops, search, etc.

use async_trait::async_trait;
use std::sync::Arc;
use crate::skills::{Skill, SkillManifest, SkillError};

/// File operations skill
pub struct FileOpsSkill;

impl FileOpsSkill {
    pub fn new() -> Self {
        Self
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

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
        match method {
            "read" => {
                let path = args.get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                std::fs::read_to_string(path)
                    .map(|content| serde_json::json!({ "content": content }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "write" => {
                let path = args.get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                let content = args.get("content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("content required".to_string()))?;
                std::fs::write(path, content)
                    .map(|_| serde_json::json!({ "success": true }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "exists" => {
                let path = args.get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                Ok(serde_json::json!({ "exists": std::path::Path::new(path).exists() }))
            }
            "delete" => {
                let path = args.get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("path required".to_string()))?;
                std::fs::remove_file(path)
                    .map(|_| serde_json::json!({ "success": true }))
                    .map_err(|e| SkillError::ExecutionFailed(e.to_string()))
            }
            "list" => {
                let path = args.get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
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
            })
        }
    }
}

/// Git operations skill
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

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
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
                let message = args.get("message")
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
            })
        }
    }
}

/// Search skill (web search)
pub struct SearchSkill;

impl SearchSkill {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Skill for SearchSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "search".to_string(),
            version: "1.0.0".to_string(),
            description: "Web search capabilities".to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["search"]
    }

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
        match method {
            "search" => {
                let query = args.get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("query required".to_string()))?;
                // Stub - would integrate with search API
                Ok(serde_json::json!({
                    "query": query,
                    "results": [],
                    "message": "Search skill stub - integrate with search API"
                }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "search".to_string(),
                method: method.to_string(),
            })
        }
    }
}

/// Permission skill - allows agents to query their own permissions
pub struct PermissionSkill {
    /// Reference to the permission engine (set at construction)
    engine: Option<Arc<crate::permission::PermissionEngine>>,
}

impl PermissionSkill {
    pub fn new() -> Self {
        Self { engine: None }
    }

    /// Create a new PermissionSkill with a permission engine reference
    pub fn with_engine(engine: Arc<crate::permission::PermissionEngine>) -> Self {
        Self { engine: Some(engine) }
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
            description: "Query the current agent's permission configuration. " .to_string()
                + "Supported actions: exec, file_read, file_write, network, spawn, tool_call, config_write",
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["query", "list_actions"]
    }

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
        match method {
            "query" => {
                let agent_id = args.get("agent_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("agent_id required".to_string()))?;
                let action = args.get("action")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("action required".to_string()))?;

                if let Some(ref engine) = self.engine {
                    let response = engine.check(agent_id, action);
                    match response {
                        crate::permission::PermissionResponse::Allowed { token: _ } => {
                            Ok(serde_json::json!({
                                "allowed": true,
                                "agent_id": agent_id,
                                "action": action,
                            }))
                        }
                        crate::permission::PermissionResponse::Denied { reason, rule: _ } => {
                            Ok(serde_json::json!({
                                "allowed": false,
                                "agent_id": agent_id,
                                "action": action,
                                "reason": reason,
                            }))
                        }
                    }
                } else {
                    // No engine available - return unknown
                    Ok(serde_json::json!({
                        "allowed": null,
                        "agent_id": agent_id,
                        "action": action,
                        "reason": "permission engine not available",
                    }))
                }
            }
            "list_actions" => {
                Ok(serde_json::json!({
                    "actions": [
                        "exec",
                        "file_read",
                        "file_write",
                        "network",
                        "spawn",
                        "tool_call",
                        "config_write",
                    ]
                }))
            }
            _ => Err(SkillError::MethodNotFound {
                skill: "permission_query".to_string(),
                method: method.to_string(),
            })
        }
    }
}


/// Skill discovery skill - allows agents to search and install skills from ClawHub
pub struct SkillDiscoverySkill;

impl SkillDiscoverySkill {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Skill for SkillDiscoverySkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_discovery".to_string(),
            version: "1.0.0".to_string(),
            description: "Search, install, and manage skills from ClawHub marketplace. "
                + "Use find to search, install to add, list to see installed, update to upgrade.",
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec!["clawhub".to_string()],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["find", "install", "list", "update"]
    }

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
        match method {
            "find" => {
                let query = args.get("query").and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("query required".to_string()))?;
                let output = tokio::process::Command::new("clawhub").args(["search", query])
                    .output().await
                    .map_err(|e| SkillError::ExecutionFailed(format!("clawhub search failed: {}", e)))?;
                Ok(serde_json::json!({"query": query, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}))
            }
            "install" => {
                let skill = args.get("skill").and_then(|v| v.as_str())
                    .ok_or_else(|| SkillError::InvalidArgs("skill name required".to_string()))?;
                let version = args.get("version").and_then(|v| v.as_str());
                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["install", skill]);
                if let Some(v) = version { cmd.arg("--version").arg(v); }
                let output = cmd.output().await
                    .map_err(|e| SkillError::ExecutionFailed(format!("clawhub install failed: {}", e)))?;
                Ok(serde_json::json!({"skill": skill, "version": version, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}))
            }
            "list" => {
                let output = tokio::process::Command::new("clawhub").args(["list"])
                    .output().await
                    .map_err(|e| SkillError::ExecutionFailed(format!("clawhub list failed: {}", e)))?;
                Ok(serde_json::json!({"output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}))
            }
            "update" => {
                let skill = args.get("skill").and_then(|v| v.as_str());
                let mut cmd = tokio::process::Command::new("clawhub");
                cmd.args(["update"]);
                if let Some(s) = skill { cmd.arg(s); } else { cmd.arg("--all"); }
                let output = cmd.output().await
                    .map_err(|e| SkillError::ExecutionFailed(format!("clawhub update failed: {}", e)))?;
                Ok(serde_json::json!({"skill": skill, "output": String::from_utf8_lossy(&output.stdout),
                    "error": if output.status.success() { None } else { Some(String::from_utf8_lossy(&output.stderr).to_string()) }}))
            }
            _ => Err(SkillError::MethodNotFound { skill: "skill_discovery".to_string(), method: method.to_string() })
        }
    }
}

/// Built-in skills registry
pub struct BuiltinSkills;

impl BuiltinSkills {
    pub fn all() -> Vec<Arc<dyn Skill>> {
        vec![
            Arc::new(FileOpsSkill::new()) as Arc<dyn Skill>,
            Arc::new(GitOpsSkill::new()),
            Arc::new(SearchSkill::new()),
            Arc::new(PermissionSkill::new()),
            Arc::new(SkillDiscoverySkill::new()),
            Arc::new(super::CodingAgentSkill::new(None)),
            Arc::new(super::SkillCreatorSkill::new()),
        ]
    }
}

/// Get all built-in skills
pub fn builtin_skills() -> Vec<Arc<dyn Skill>> {
    BuiltinSkills::all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_file_ops_read() {
        let skill = FileOpsSkill::new();
        let result = skill.execute("read", serde_json::json!({"path": "Cargo.toml"})).await;
        assert!(result.is_ok());
        let value = result.unwrap();
        assert!(value.get("content").is_some());
    }

    #[tokio::test]
    async fn test_file_ops_exists() {
        let skill = FileOpsSkill::new();
        let result = skill.execute("exists", serde_json::json!({"path": "Cargo.toml"})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_git_ops_status() {
        let skill = GitOpsSkill::new();
        let result = skill.execute("status", serde_json::json!({})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search() {
        let skill = SearchSkill::new();
        let result = skill.execute("search", serde_json::json!({"query": "rust programming"})).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_skills() {
        let skills = BuiltinSkills::all();
        assert_eq!(skills.len(), 7);
        assert_eq!(skills[0].manifest().name, "file_ops");
        assert_eq!(skills[1].manifest().name, "git_ops");
        assert_eq!(skills[2].manifest().name, "search");
        assert_eq!(skills[3].manifest().name, "permission_query");
        assert_eq!(skills[4].manifest().name, "coding_agent");
        assert_eq!(skills[5].manifest().name, "skill_creator");
    }

    // From tests/smoke_test.rs
    #[tokio::test]
    async fn test_skill_registry_with_builtins() {
        use crate::skills::SkillRegistry;
        let registry = SkillRegistry::new();
        for skill in builtin_skills() {
            registry.register(skill).await;
        }
        let skills: Vec<String> = registry.list().await;
        assert!(skills.contains(&"file_ops".to_string()));
        assert!(skills.contains(&"git_ops".to_string()));
        assert!(skills.contains(&"search".to_string()));
    }
}
