//! Skill Creator Skill - Help agents create new skills
//!
//! This skill helps agents understand how to create new skills for CloseClaw.

use crate::skills::{Skill, SkillError, SkillManifest};
use async_trait::async_trait;

pub struct SkillCreatorSkill;

const GUIDE_JSON: &str = r##"{"content":"# Creating a CloseClaw Skill\\n\\n## 1. Create the Skill File\\nCreate `src/skills/your_skill_name.rs`:\\n\\n```rust\\nuse async_trait::async_trait;\\nuse crate::skills::{Skill, SkillManifest, SkillError};\\n\\npub struct YourSkill;\\n\\nimpl YourSkill {\\n    pub fn new() -> Self { Self }\\n}\\n\\n#[async_trait]\\nimpl Skill for YourSkill {\\n    fn manifest(&self) -> SkillManifest {\\n        SkillManifest {\\n            name: \"your_skill_name\".to_string(),\\n            version: \"1.0.0\".to_string(),\\n            description: \"What your skill does\".to_string(),\\n            author: Some(\"Your Name\".to_string()),\\n            dependencies: vec![],\\n        }\\n    }\\n\\n    fn methods(&self) -> Vec<&str> {\\n        vec![\"method1\", \"method2\"]\\n    }\\n\\n    async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {\\n        match method {\\n            \"method1\" => { /* ... */ }\\n            _ => Err(SkillError::MethodNotFound { ... })\\n        }\\n    }\\n}\\n```\\n\\n## 2. Register in mod.rs\\n```rust\\npub mod your_skill_name;\\npub use your_skill_name::YourSkill;\\n```\\n\\n## 3. Create SKILL.md Documentation\\nCreate `docs/skills/your_skill_name/SKILL.md` following the standard format.\\n","format":"markdown"}"##;

const TEMPLATE_JSON: &str = r##"{"template":"---\\nname: skill-name\\ndescription: |\\n  One-line description of what this skill does.\\n---\\n\\n# Skill Name\\n\\n## Overview\\nDescription of the skill.\\n\\n## Quick Reference\\n| User Intent | Tool | action | Required |\\n|-------------|------|--------|----------|\\n| ... | ... | ... | ... |\\n\\n## Usage\\nDetailed usage instructions.\\n","format":"markdown"}"##;

impl SkillCreatorSkill {
    pub fn new() -> Self {
        Self
    }

    fn execute_guide() -> Result<serde_json::Value, SkillError> {
        serde_json::from_str(GUIDE_JSON)
            .map_err(|e| SkillError::InvalidArgs(format!("invalid built-in guide JSON: {}", e)))
    }

    fn execute_template() -> Result<serde_json::Value, SkillError> {
        serde_json::from_str(TEMPLATE_JSON)
            .map_err(|e| SkillError::InvalidArgs(format!("invalid built-in template JSON: {}", e)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_guide_returns_valid_json() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute("guide", serde_json::Value::Null).await;
        assert!(result.is_ok(), "guide should succeed: {:?}", result);
        let value = result.unwrap();
        let content = value
            .get("content")
            .and_then(|v| v.as_str())
            .expect("content field should be a string");
        assert!(
            content.contains("Creating a CloseClaw Skill"),
            "guide content should mention 'Creating a CloseClaw Skill'"
        );
    }

    #[tokio::test]
    async fn test_template_returns_valid_json() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute("template", serde_json::Value::Null).await;
        assert!(result.is_ok(), "template should succeed: {:?}", result);
        let value = result.unwrap();
        let template = value.get("template");
        assert!(
            template.is_some() && !template.unwrap().is_null(),
            "template field should be non-null"
        );
    }

    #[tokio::test]
    async fn test_validate_valid_code() {
        let skill = SkillCreatorSkill::new();
        let valid_code = r#"
            use async_trait::async_trait;
            use crate::skills::{Skill, SkillManifest, SkillError};

            pub struct MySkill;

            impl MySkill {
                pub fn new() -> Self { Self }
            }

            #[async_trait]
            impl Skill for MySkill {
                fn manifest(&self) -> SkillManifest {
                    SkillManifest { name: "".into(), version: "".into(), description: "".into(), author: None, dependencies: vec![] }
                }
                fn methods(&self) -> Vec<&str> { vec![] }
                async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
                    Ok(serde_json::Value::Null)
                }
            }
        "#;
        let args = serde_json::json!({ "code": valid_code });
        let result = skill.execute("validate", args).await;
        assert!(result.is_ok(), "validate should succeed: {:?}", result);
        let value = result.unwrap();
        let valid = value
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(valid, "valid code should pass validation: {}", value);
        let checks = value.get("checks").expect("checks field should exist");
        assert_eq!(
            checks.get("has_async_trait_impl").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            checks.get("has_manifest").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            checks.get("has_execute").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            checks.get("has_methods").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn test_validate_missing_async_trait() {
        let skill = SkillCreatorSkill::new();
        let invalid_code = r#"
            use crate::skills::{Skill, SkillManifest, SkillError};

            pub struct MySkill;

            impl MySkill {
                pub fn new() -> Self { Self }
            }

            impl Skill for MySkill {
                fn manifest(&self) -> SkillManifest {
                    SkillManifest { name: "".into(), version: "".into(), description: "".into(), author: None, dependencies: vec![] }
                }
                fn methods(&self) -> Vec<&str> { vec![] }
                async fn execute(&self, method: &str, args: serde_json::Value) -> Result<serde_json::Value, SkillError> {
                    Ok(serde_json::Value::Null)
                }
            }
        "#;
        let args = serde_json::json!({ "code": invalid_code });
        let result = skill.execute("validate", args).await;
        assert!(
            result.is_ok(),
            "validate should still succeed even with invalid code"
        );
        let value = result.unwrap();
        let valid = value.get("valid").and_then(|v| v.as_bool()).unwrap_or(true);
        assert!(
            !valid,
            "code missing async_trait should be invalid: {}",
            value
        );
    }

    #[tokio::test]
    async fn test_validate_missing_code_field() {
        let skill = SkillCreatorSkill::new();
        let result = skill
            .execute("validate", serde_json::Value::Object(Default::default()))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            SkillError::InvalidArgs(_) => {}
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_unknown_method_returns_method_not_found() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute("nonexistent", serde_json::Value::Null).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            SkillError::MethodNotFound {
                ref skill,
                ref method,
            } => {
                assert_eq!(skill, "skill_creator");
                assert_eq!(method, "nonexistent");
            }
            other => panic!("expected MethodNotFound, got {:?}", other),
        }
    }
}
