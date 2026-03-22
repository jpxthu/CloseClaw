# Skill Architecture Overview

## Skill Trait
```rust
use async_trait::async_trait;
use crate::skills::{Skill, SkillManifest, SkillError};

pub trait Skill: Send + Sync {
    fn manifest(&self) -> SkillManifest;
    fn methods(&self) -> Vec<&str>;
    async fn execute(&self, method: &str, args: Value) -> Result<Value, SkillError>;
}
```

## SkillManifest
```rust
pub struct SkillManifest {
    pub name: String,          // Unique identifier
    pub version: String,        // Semver (e.g., "1.0.0")
    pub description: String,    // One-line description
    pub author: Option<String>,
    pub dependencies: Vec<String>,
}
```

## SkillError Variants
```rust
pub enum SkillError {
    NotFound(String),                    // Skill not found
    MethodNotFound { skill, method },   // Method not implemented
    ExecutionFailed(String),            // Execution error
    InvalidArgs(String),                // Invalid arguments
    PermissionDenied(String),           // Permission denied
}
```

## SkillRegistry
```rust
pub struct SkillRegistry {
    skills: RwLock<HashMap<String, Arc<dyn Skill>>>,
}

impl SkillRegistry {
    pub async fn register(&self, skill: Arc<dyn Skill>);
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Skill>>;
    pub async fn list(&self) -> Vec<String>;
}
```

## Execution Flow
1. Agent calls `registry.execute("skill_name", "method", args)`
2. Registry looks up skill by name
3. Registry calls `skill.execute(method, args)`
4. Skill returns `Result<Value, SkillError>`
