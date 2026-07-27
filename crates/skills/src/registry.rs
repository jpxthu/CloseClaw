//! Skill Registry - manages skill registration and discovery

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::disk::types::SkillEffort;

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

/// Metadata required for listing generation.
///
/// Builtin skills provide this so they can appear in the same
/// skill listing that disk-based skills already produce.
#[derive(Debug, Clone)]
pub struct SkillListingMeta {
    /// When to use this skill (decision hint).
    pub when_to_use: String,
    /// Whether the skill can be invoked directly by a user.
    pub user_invocable: bool,
    /// File glob patterns for conditional activation.
    pub paths: Vec<String>,
    /// Estimated effort level.
    pub effort: SkillEffort,
}

/// Skill trait - implemented by each skill
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get skill manifest
    fn manifest(&self) -> SkillManifest;

    /// Get skill prompt body text
    fn body(&self) -> &str;

    /// Get listing metadata for this skill.
    ///
    /// Used by the listing generator to render builtin skills
    /// into the same format as disk-based skills.
    fn listing_meta(&self) -> SkillListingMeta;
}

/// Builtin skill registry - manages all registered builtin skills
pub struct BuiltinSkillRegistry {
    skills: tokio::sync::RwLock<HashMap<String, Arc<dyn Skill>>>,
}

impl Default for BuiltinSkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinSkillRegistry {
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

    /// Create a registry pre-populated with the given skills.
    pub async fn from_skills(skills: Vec<Arc<dyn Skill>>) -> Self {
        let registry = Self::new();
        for skill in skills {
            registry.register(skill).await;
        }
        registry
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill '{0}' not found")]
    NotFound(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::disk::types::SkillEffort;

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

        fn body(&self) -> &str {
            "mock body"
        }

        fn listing_meta(&self) -> SkillListingMeta {
            SkillListingMeta {
                when_to_use: format!("use {} when needed", self.name),
                user_invocable: false,
                paths: vec![],
                effort: SkillEffort::Unknown,
            }
        }
    }

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = BuiltinSkillRegistry::new();
        let skill = Arc::new(MockSkill::new("test_skill"));
        registry.register(skill).await;

        let found = registry.get("test_skill").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().manifest().name, "test_skill");
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let registry = BuiltinSkillRegistry::new();
        let found = registry.get("nonexistent").await;
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_list() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill_a"))).await;
        registry.register(Arc::new(MockSkill::new("skill_b"))).await;

        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["skill_a", "skill_b"]);
    }

    #[tokio::test]
    async fn test_contains() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("exists"))).await;

        assert!(registry.contains("exists").await);
        assert!(!registry.contains("missing").await);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("to_remove")))
            .await;

        assert!(registry.unregister("to_remove").await);
        assert!(!registry.contains("to_remove").await);
        assert!(!registry.unregister("to_remove").await);
    }

    #[tokio::test]
    async fn test_register_replaces() {
        let registry = BuiltinSkillRegistry::new();
        registry.register(Arc::new(MockSkill::new("skill"))).await;
        registry.register(Arc::new(MockSkill::new("skill"))).await;

        let names = registry.list().await;
        assert_eq!(names.len(), 1);
    }

    #[tokio::test]
    async fn test_body_returns_value() {
        let registry = BuiltinSkillRegistry::new();
        registry
            .register(Arc::new(MockSkill::new("body_skill")))
            .await;

        let skill = registry.get("body_skill").await.unwrap();
        assert_eq!(skill.body(), "mock body");
    }

    #[tokio::test]
    async fn test_skill_error_display() {
        let err = SkillError::NotFound("test".to_string());
        assert!(err.to_string().contains("test"));

        let err = SkillError::ExecutionFailed("boom".to_string());
        assert!(err.to_string().contains("boom"));

        let err = SkillError::InvalidArgs("bad".to_string());
        assert!(err.to_string().contains("bad"));
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
    fn test_registry_default() {
        let registry = BuiltinSkillRegistry::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let names = registry.list().await;
            assert!(names.is_empty());
        });
    }

    #[test]
    fn test_mock_listing_meta() {
        let skill = MockSkill::new("test");
        let meta = skill.listing_meta();
        assert_eq!(meta.when_to_use, "use test when needed");
        assert!(!meta.user_invocable);
        assert!(meta.paths.is_empty());
        assert_eq!(meta.effort, SkillEffort::Unknown);
    }

    #[tokio::test]
    async fn test_from_skills_registers_all() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("alpha")),
            Arc::new(MockSkill::new("beta")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(registry.contains("alpha").await);
        assert!(registry.contains("beta").await);
    }

    #[tokio::test]
    async fn test_from_skills_empty() {
        let registry = BuiltinSkillRegistry::from_skills(vec![]).await;
        assert!(registry.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_from_skills_overwrites_duplicates() {
        let skills: Vec<Arc<dyn Skill>> = vec![
            Arc::new(MockSkill::new("dup")),
            Arc::new(MockSkill::new("dup")),
        ];
        let registry = BuiltinSkillRegistry::from_skills(skills).await;
        let names = registry.list().await;
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "dup");
    }
}
