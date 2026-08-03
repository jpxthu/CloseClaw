//! Skill Creator Skill - Help agents create new skills
//!
//! This skill helps agents create SKILL.md files for CloseClaw.

use crate::disk::types::SkillEffort;
use crate::registry::{Skill, SkillError, SkillListingMeta, SkillManifest};
use async_trait::async_trait;
use serde_json::json;

pub struct SkillCreatorSkill;

impl Default for SkillCreatorSkill {
    fn default() -> Self {
        Self
    }
}

impl SkillCreatorSkill {
    pub fn new() -> Self {
        Self
    }

    /// Build the default capability description returned when no
    /// action is specified.
    fn capabilities_description() -> String {
        json!({
            "skill": "skill_creator",
            "description": "Helps agents create, validate, and edit skill files",
            "supported_actions": ["create", "validate", "edit"],
            "usage": {
                "create": {"name": "<skill_name>", "description": "<one-line description>"},
                "validate": {"path": "<path_to_skill_md>"},
                "edit": {
                    "path": "<skill_md_path>",
                    "field": "<field_name>",
                    "value": "<new_value>"
                }
            }
        })
        .to_string()
    }

    /// Return structured guidance for creating a new skill.
    fn build_create_guidance(name: &str, description: &str) -> String {
        json!({
            "skill": "skill_creator",
            "action": "create",
            "target": {
                "name": name,
                "file": format!("skills/{name}/SKILL.md"),
                "description": description
            },
            "template": {
                "frontmatter": {
                    "description": description,
                    "when-to-use": format!("Use when the agent needs to {name}"),
                    "user-invocable": true,
                    "effort": "small"
                },
                "body_outline": [
                    "# Skill Name",
                    "",
                    "## Overview",
                    "Description of the skill purpose and capabilities.",
                    "",
                    "## Instructions",
                    "Step-by-step instructions for the Agent to follow."
                ]
            },
            "instructions": "Use `write` to create. Frontmatter needs `---` and `description`."
        })
        .to_string()
    }

    /// Return structured guidance for validating a skill file.
    fn build_validate_guidance(path: &str) -> String {
        json!({
            "skill": "skill_creator",
            "action": "validate",
            "target": {"path": path},
            "checks": [
                "File exists and is readable",
                "Has `---` frontmatter delimiters at the top",
                "Frontmatter contains a `description` field",
                "YAML syntax in frontmatter is valid",
                "Markdown content exists after the frontmatter"
            ],
            "instructions": "Use the `read` tool to read the file, then verify each check."
        })
        .to_string()
    }

    /// Return structured guidance for editing a skill file.
    fn build_edit_guidance(path: &str, field: &str, value: &str) -> String {
        json!({
            "skill": "skill_creator",
            "action": "edit",
            "target": {"path": path},
            "change": {
                "field": field,
                "value": value
            },
            "instructions": "Use `read` to load, modify frontmatter, then `write` to save."
        })
        .to_string()
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

    async fn execute(&self, args: Option<serde_json::Value>) -> Result<String, SkillError> {
        let args = match args {
            Some(a) => a,
            None => return Ok(Self::capabilities_description()),
        };

        let action = args.get("action").and_then(|v| v.as_str());

        match action {
            None => Ok(Self::capabilities_description()),
            Some("create") => {
                let name = args.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'name' for create action".into())
                })?;
                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("New skill");
                Ok(Self::build_create_guidance(name, description))
            }
            Some("validate") => {
                let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for validate action".into())
                })?;
                Ok(Self::build_validate_guidance(path))
            }
            Some("edit") => {
                let path = args.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'path' for edit action".into())
                })?;
                let field = args.get("field").and_then(|v| v.as_str()).ok_or_else(|| {
                    SkillError::InvalidArgs("missing 'field' for edit action".into())
                })?;
                let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                Ok(Self::build_edit_guidance(path, field, value))
            }
            Some(other) => Err(SkillError::InvalidArgs(format!(
                "unknown action '{other}', supported: create, validate, edit"
            ))),
        }
    }

    fn body(&self) -> &str {
        r#"# Skill Creator

Use the `write` tool to create a SKILL.md file. The file must follow this format:

## SKILL.md Template

```markdown
---
description: One-line description of what this skill does.
when-to-use: Decision hints for when to invoke this skill.
paths:
  - "src/**/*.rs"
user-invocable: true
effort: small
---

# Skill Name

## Overview
Description of the skill purpose and capabilities.

## Instructions
Step-by-step instructions for the Agent to follow when this skill is invoked.

Use `${SKILL_DIR}` to reference the skill directory path (disk-based skills only).
Use `${SESSION_ID}` to reference the current session ID (disk-based skills only).
```

## Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| description | Yes | Brief skill description for Agent decision-making |
| when-to-use | No | Decision hints for invocation timing |
| paths | No | File glob patterns for conditional activation |
| user-invocable | No | Allow direct user invocation via slash commands |
| effort | No | Cost estimate: small, medium, large |

## Validation

A valid SKILL.md must:
1. Have a `---` frontmatter block at the top
2. Include a `description` field in frontmatter
3. Have Markdown content after the frontmatter
4. Use proper YAML syntax in frontmatter"#
    }

    fn listing_meta(&self) -> SkillListingMeta {
        SkillListingMeta {
            when_to_use: "Use when the agent needs to create or understand \
                how to create new skills for CloseClaw"
                .to_string(),
            user_invocable: true,
            paths: vec![],
            effort: SkillEffort::Small,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest() {
        let skill = SkillCreatorSkill::new();
        let m = skill.manifest();
        assert_eq!(m.name, "skill_creator");
        assert_eq!(m.version, "1.0.0");
        assert!(!m.description.is_empty());
    }

    #[test]
    fn test_body_not_empty() {
        let skill = SkillCreatorSkill::new();
        let body = skill.body();
        assert!(!body.is_empty());
        assert!(body.contains("Skill Creator"));
        assert!(body.contains("SKILL.md Template"));
    }

    #[test]
    fn test_body_contains_frontmatter_guide() {
        let skill = SkillCreatorSkill::new();
        let body = skill.body();
        assert!(body.contains("description"));
        assert!(body.contains("Frontmatter Fields"));
    }

    #[test]
    fn test_default() {
        let skill = SkillCreatorSkill::default();
        assert_eq!(skill.manifest().name, "skill_creator");
    }

    // --- execute() tests ---

    #[tokio::test]
    async fn test_execute_none_returns_capabilities() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute(None).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
        let actions = v["supported_actions"].as_array().unwrap();
        assert!(actions.contains(&json!("create")));
        assert!(actions.contains(&json!("validate")));
        assert!(actions.contains(&json!("edit")));
    }

    #[tokio::test]
    async fn test_execute_empty_args_returns_capabilities() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute(Some(json!({}))).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
    }

    #[tokio::test]
    async fn test_execute_no_action_returns_capabilities() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute(Some(json!({"name": "test"}))).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
        assert!(v["supported_actions"].is_array());
    }

    #[tokio::test]
    async fn test_execute_create_with_name_and_description() {
        let skill = SkillCreatorSkill::new();
        let result = skill
            .execute(Some(json!({
                "action": "create",
                "name": "my_skill",
                "description": "Does cool things"
            })))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
        assert_eq!(v["action"], "create");
        assert_eq!(v["target"]["name"], "my_skill");
        assert_eq!(v["target"]["description"], "Does cool things");
        assert!(v["template"]["frontmatter"].is_object());
        assert!(v["instructions"].is_string());
    }

    #[tokio::test]
    async fn test_execute_create_without_description() {
        let skill = SkillCreatorSkill::new();
        let result = skill
            .execute(Some(json!({"action": "create", "name": "foo"})))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["action"], "create");
        assert_eq!(v["target"]["description"], "New skill");
    }

    #[tokio::test]
    async fn test_execute_create_missing_name() {
        let skill = SkillCreatorSkill::new();
        let err = skill
            .execute(Some(json!({"action": "create"})))
            .await
            .unwrap_err();
        match err {
            crate::registry::SkillError::InvalidArgs(msg) => {
                assert!(msg.contains("name"));
            }
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_validate() {
        let skill = SkillCreatorSkill::new();
        let result = skill
            .execute(Some(json!({
                "action": "validate",
                "path": "skills/test/SKILL.md"
            })))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
        assert_eq!(v["action"], "validate");
        assert_eq!(v["target"]["path"], "skills/test/SKILL.md");
        let checks = v["checks"].as_array().unwrap();
        assert!(!checks.is_empty());
    }

    #[tokio::test]
    async fn test_execute_validate_missing_path() {
        let skill = SkillCreatorSkill::new();
        let err = skill
            .execute(Some(json!({"action": "validate"})))
            .await
            .unwrap_err();
        match err {
            crate::registry::SkillError::InvalidArgs(msg) => {
                assert!(msg.contains("path"));
            }
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_edit() {
        let skill = SkillCreatorSkill::new();
        let result = skill
            .execute(Some(json!({
                "action": "edit",
                "path": "skills/test/SKILL.md",
                "field": "description",
                "value": "Updated desc"
            })))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["skill"], "skill_creator");
        assert_eq!(v["action"], "edit");
        assert_eq!(v["target"]["path"], "skills/test/SKILL.md");
        assert_eq!(v["change"]["field"], "description");
        assert_eq!(v["change"]["value"], "Updated desc");
    }

    #[tokio::test]
    async fn test_execute_edit_missing_field() {
        let skill = SkillCreatorSkill::new();
        let err = skill
            .execute(Some(json!({"action": "edit", "path": "x"})))
            .await
            .unwrap_err();
        match err {
            crate::registry::SkillError::InvalidArgs(msg) => {
                assert!(msg.contains("field"));
            }
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_edit_missing_path() {
        let skill = SkillCreatorSkill::new();
        let err = skill
            .execute(Some(json!({
                "action": "edit",
                "field": "description",
                "value": "x"
            })))
            .await
            .unwrap_err();
        match err {
            crate::registry::SkillError::InvalidArgs(msg) => {
                assert!(msg.contains("path"));
            }
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let skill = SkillCreatorSkill::new();
        let err = skill
            .execute(Some(json!({"action": "bogus"})))
            .await
            .unwrap_err();
        match err {
            crate::registry::SkillError::InvalidArgs(msg) => {
                assert!(msg.contains("unknown action"));
            }
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_does_not_delegate_to_body() {
        let skill = SkillCreatorSkill::new();
        let result = skill.execute(None).await.unwrap();
        assert_ne!(result, skill.body());
        // Should be valid JSON
        let _: serde_json::Value = serde_json::from_str(&result).unwrap();
    }
}
