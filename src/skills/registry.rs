//! Skill Registry - manages skill registration and discovery

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Skill metadata
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// Input for skill execution
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillInput {
    pub skill_name: String,
    pub method: String,
    pub args: serde_json::Value,
    pub agent_id: String,
}

/// Output from skill execution
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillOutput {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Skill trait - implemented by each skill
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get skill manifest
    fn manifest(&self) -> SkillManifest;

    /// List available methods
    fn methods(&self) -> Vec<&str>;

    /// Execute a method
    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError>;
}

/// Skill registry - manages all registered skills
pub struct SkillRegistry {
    skills: tokio::sync::RwLock<HashMap<String, Arc<dyn Skill>>>,
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a skill
    pub async fn register(&self, skill: Arc<dyn Skill>) {
        let mut skills = self.skills.write().await;
        skills.insert(skill.manifest().name.clone(), skill);
    }

    /// Get a skill by name
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        let skills = self.skills.read().await;
        skills.get(name).cloned()
    }

    /// List all skill names
    pub async fn list(&self) -> Vec<String> {
        let skills = self.skills.read().await;
        skills.keys().cloned().collect()
    }

    /// Check if a skill exists
    pub async fn contains(&self, name: &str) -> bool {
        let skills = self.skills.read().await;
        skills.contains_key(name)
    }

    /// Unregister a skill
    pub async fn unregister(&self, name: &str) -> bool {
        let mut skills = self.skills.write().await;
        skills.remove(name).is_some()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill '{0}' not found")]
    NotFound(String),

    #[error("Method '{method}' not found in skill '{skill}'")]
    MethodNotFound { skill: String, method: String },

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSkill {
        name: String,
    }

    impl MockSkill {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl Skill for MockSkill {
        fn manifest(&self) -> SkillManifest {
            SkillManifest {
                name: self.name.clone(),
                version: "1.0.0".to_string(),
                description: format!("mock skill {}", self.name),
                author: None,
                dependencies: vec![],
            }
        }

        fn methods(&self) -> Vec<&str> {
            vec!["method1", "method2"]
        }

        async fn execute(
            &self,
            method: &str,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, SkillError> {
            match method {
                "method1" => Ok(serde_json::json!({"method": method})),
                _ => Err(SkillError::MethodNotFound {
                    skill: self.name.clone(),
                    method: method.to_string(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = SkillRegistry::new();
        let skill = Arc::new(MockSkill::new("test_skill"));
        registry.register(skill).await;

        let found = registry.get("test_skill").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().manifest().name, "test_skill");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let registry = SkillRegistry::new();
        let found = registry.get("nonexistent").await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_list() {
        let registry = SkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill_a"))).await;
        registry.register(Arc::new(MockSkill::new("skill_b"))).await;

        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["skill_a", "skill_b"]);
    }

    #[tokio::test]
    async fn test_contains() {
        let registry = SkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("exists"))).await;

        assert!(registry.contains("exists").await);
        assert!(!registry.contains("missing").await);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = SkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("to_remove")))
            .await;

        assert!(registry.unregister("to_remove").await);
        assert!(!registry.contains("to_remove").await);
        assert!(!registry.unregister("to_remove").await);
    }

    #[tokio::test]
    async fn test_register_replaces() {
        let registry = SkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill"))).await;
        registry.register(Arc::new(MockSkill::new("skill"))).await;

        let names = registry.list().await;
        assert_eq!(names.len(), 1);
    }

    #[tokio::test]
    async fn test_execute_method() {
        let registry = SkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("exec_skill")))
            .await;

        let skill = registry.get("exec_skill").await.unwrap();
        let result = skill.execute("method1", serde_json::Value::Null).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["method"], "method1");
    }

    #[tokio::test]
    async fn test_skill_error_display() {
        let err = SkillError::NotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = SkillError::ExecutionFailed("boom".to_string());
        assert!(err.to_string().contains("boom"));

        let err = SkillError::InvalidArgs("bad".to_string());
        assert!(err.to_string().contains("bad"));

        let err = SkillError::PermissionDenied("no".to_string());
        assert!(err.to_string().contains("no"));

        let err = SkillError::MethodNotFound {
            skill: "s".to_string(),
            method: "m".to_string(),
        };
        assert!(err.to_string().contains("s"));
        assert!(err.to_string().contains("m"));
    }

    #[test]
    fn test_skill_manifest_serialization() {
        let manifest = SkillManifest {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: "desc".to_string(),
            author: Some("author".to_string()),
            dependencies: vec!["dep1".to_string()],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: SkillManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.author, Some("author".to_string()));
        assert_eq!(parsed.dependencies, vec!["dep1".to_string()]);
    }

    #[test]
    fn test_skill_output_serialization() {
        let output = SkillOutput {
            success: true,
            result: Some(serde_json::json!("ok")),
            error: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        let parsed: SkillOutput = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_skill_input_serialization() {
        let input = SkillInput {
            skill_name: "skill".to_string(),
            method: "run".to_string(),
            args: serde_json::json!({"key": "value"}),
            agent_id: "agent1".to_string(),
        };
        let json = serde_json::to_string(&input).unwrap();
        let parsed: SkillInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.skill_name, "skill");
    }

    #[test]
    fn test_registry_default() {
        let registry = SkillRegistry::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let names = registry.list().await;
            assert!(names.is_empty());
        });
    }
}
