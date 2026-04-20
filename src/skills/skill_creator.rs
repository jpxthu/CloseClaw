//! Skill Creator Skill - Help agents create new skills
//!
//! This skill helps agents understand how to create new skills for CloseClaw.

use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;

pub struct SkillCreatorSkill;

const GUIDE_JSON: &str = r##"{"content":"# Creating a CloseClaw Skill

## 1. Create the Skill File
Create `src/skills/your_skill_name.rs`:

```rust
use async_trait::async_trait;
use crate::skills::{Skill, SkillManifest, SkillError};

pub struct YourSkill;

impl YourSkill {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Skill for YourSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "your_skill_name".to_string(),
            version: "1.0.0".to_string(),
            description: "What your skill does".to_string(),
            author: Some("Your Name".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["method1", "method2"]
    }

    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
        match method {
            "method1" => { /* ... */ }
            _ => Err(SkillError::MethodNotFound { ... })
        }
    }
}
```

## 2. Register in mod.rs
```rust
pub mod your_skill_name;
pub use your_skill_name::YourSkill;
```

## 3. Create SKILL.md Documentation
Create `docs/skills/your_skill_name/SKILL.md` following the standard format.
","format":"markdown"}"##;

const TEMPLATE_JSON: &str = r##"{"template":"---
name: skill-name
description: |
  One-line description of what this skill does.
---

# Skill Name

## Overview
Description of the skill.

## Quick Reference
| User Intent | Tool | action | Required |
|-------------|------|--------|----------|
| ... | ... | ... | ... |

## Usage
Detailed usage instructions.
","format":"markdown"}"##;

impl SkillCreatorSkill {
    pub fn new() -> Self {
        Self
    }

    fn execute_guide() -> Result<serde_json::Value, SkillError> {
        serde_json::from_str(GUIDE_JSON).map_err(|e| SkillError::ExecutionFailed(e.to_string()))
    }

    fn execute_template() -> Result<serde_json::Value, SkillError> {
        serde_json::from_str(TEMPLATE_JSON).map_err(|e| SkillError::ExecutionFailed(e.to_string()))
    }

    fn execute_validate(args: &serde_json::Value) -> Result<serde_json::Value, SkillError> {
        let code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SkillError::InvalidArgs("code required".to_string()))?;

        let has_async_trait = code.contains("#[async_trait]");
        let has_manifest = code.contains("fn manifest");
        let has_execute = code.contains("async fn execute");
        let has_methods = code.contains("fn methods");

        Ok(serde_json::json!({
            "valid": has_async_trait && has_manifest && has_execute && has_methods,
            "checks": {
                "has_async_trait_impl": has_async_trait,
                "has_manifest": has_manifest,
                "has_execute": has_execute,
                "has_methods": has_methods
            }
        }))
    }
}

#[async_trait]
impl Skill for SkillCreatorSkill {
    fn manifest(&self) -> SkillManifest {
        SkillManifest {
            name: "skill_creator".to_string(),
            version: "1.0.0".to_string(),
            description: "Helps agents understand how to create new skills for CloseClaw"
                .to_string(),
            author: Some("CloseClaw Team".to_string()),
            dependencies: vec![],
        }
    }

    fn methods(&self) -> Vec<&str> {
        vec!["guide", "template", "validate"]
    }

    async fn execute(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SkillError> {
        match method {
            "guide" => Self::execute_guide(),
            "template" => Self::execute_template(),
            "validate" => Self::execute_validate(&args),
            _ => Err(SkillError::MethodNotFound {
                skill: "skill_creator".to_string(),
                method: method.to_string(),
            }),
        }
    }
}
