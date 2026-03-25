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
