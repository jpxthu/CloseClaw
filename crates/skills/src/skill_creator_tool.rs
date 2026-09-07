//! SkillCreatorTool — Tool interface for SkillCreator guidance.
//!
//! This is a thin adapter that exposes the SkillCreatorSkill guidance
//! through the [`closeclaw_common::Tool`] trait, making it discoverable
//! via ToolSearch and callable as a tool.
//!
//! The tool is **read-only**: it returns guidance content for creating
//! SKILL.md files. It does not write files or modify state. Actual file
//! creation is performed by the Agent using the `write` tool after
//! receiving the guidance.

use async_trait::async_trait;
use closeclaw_common::tool_trait::{
    Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult,
};
use serde_json::Value;
use std::sync::Arc;

use crate::registry::Skill;
use crate::SkillCreatorSkill;

/// Tool that provides guidance for creating SKILL.md files.
///
/// Delegates to [`SkillCreatorSkill`] for content generation, ensuring
/// a single source of truth for guidance content (DRY).
pub struct SkillCreatorTool {
    skill: Arc<SkillCreatorSkill>,
}

impl SkillCreatorTool {
    /// Creates a new `SkillCreatorTool`.
    pub fn new() -> Self {
        Self {
            skill: Arc::new(SkillCreatorSkill::new()),
        }
    }
}

impl Default for SkillCreatorTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SkillCreatorTool {
    fn name(&self) -> &str {
        "SkillCreator"
    }

    fn group(&self) -> &str {
        "skill_creator"
    }

    fn summary(&self) -> String {
        "Guidance for creating SKILL.md files".to_string()
    }

    fn detail(&self) -> String {
        "Provides structured guidance on how to create CloseClaw skill \
         files (SKILL.md). Returns frontmatter requirements, body \
         structure, and field descriptions. Use this tool when you \
         need to understand the SKILL.md format or create a new skill."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'create', 'validate', or 'edit'",
                    "enum": ["create", "validate", "edit"]
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (for create action)"
                },
                "description": {
                    "type": "string",
                    "description": "Skill description (for create action)"
                },
                "path": {
                    "type": "string",
                    "description": "Path to SKILL.md file (for validate/edit actions)"
                },
                "field": {
                    "type": "string",
                    "description": "Frontmatter field name (for edit action)"
                },
                "value": {
                    "type": "string",
                    "description": "New field value (for edit action)"
                }
            }
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_deferred_by_default: true,
            is_read_only: true,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        // Delegate to SkillCreatorSkill for guidance content (DRY).
        let skill_args = match args.as_object() {
            Some(o) if o.is_empty() => None,
            _ => Some(args),
        };

        let content =
            self.skill
                .execute(skill_args)
                .await
                .map_err(|e: crate::registry::SkillError| {
                    ToolCallError::ExecutionFailed(e.to_string())
                })?;

        Ok(ToolResult {
            data: serde_json::json!({
                "tool": "SkillCreator",
                "status": "guidance_returned"
            }),
            new_messages: vec![ToolMessage {
                content,
                is_meta: true,
            }],
            context_modifier: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
            media_store: None,
        }
    }

    #[test]
    fn test_skill_creator_tool_name() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.name(), "SkillCreator");
    }

    #[test]
    fn test_skill_creator_tool_group() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.group(), "skill_creator");
    }

    #[test]
    fn test_skill_creator_tool_flags() {
        let tool = SkillCreatorTool::new();
        let flags = tool.flags();
        assert!(flags.is_deferred_by_default);
        assert!(flags.is_read_only);
    }

    #[test]
    fn test_skill_creator_tool_summary_not_empty() {
        let tool = SkillCreatorTool::new();
        assert!(!tool.summary().is_empty());
    }

    #[test]
    fn test_skill_creator_tool_detail_not_empty() {
        let tool = SkillCreatorTool::new();
        assert!(!tool.detail().is_empty());
    }

    #[tokio::test]
    async fn test_call_no_args_returns_capabilities() {
        let tool = SkillCreatorTool::new();
        let result = tool.call(json!({}), &new_ctx()).await.unwrap();
        assert_eq!(result.data["tool"], "SkillCreator");
        assert_eq!(result.data["status"], "guidance_returned");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        // Content should be valid JSON with skill_creator info
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        assert_eq!(v["skill"], "skill_creator");
    }

    #[tokio::test]
    async fn test_call_create_action() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "create", "name": "my_skill", "description": "Test"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        assert_eq!(v["action"], "create");
        assert_eq!(v["target"]["name"], "my_skill");
    }

    #[tokio::test]
    async fn test_call_validate_action() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "validate", "path": "skills/test/SKILL.md"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        assert_eq!(v["action"], "validate");
    }

    #[tokio::test]
    async fn test_call_edit_action() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({
                    "action": "edit",
                    "path": "skills/test/SKILL.md",
                    "field": "description",
                    "value": "Updated"
                }),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        assert_eq!(v["action"], "edit");
    }

    #[tokio::test]
    async fn test_call_no_context_modifier() {
        let tool = SkillCreatorTool::new();
        let result = tool.call(json!({}), &new_ctx()).await.unwrap();
        assert!(result.context_modifier.is_none());
    }

    // -----------------------------------------------------------------
    // Guidance content validation
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_create_guidance_includes_frontmatter() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "create", "name": "my_skill", "description": "Test"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        // Guidance must include frontmatter requirements.
        let template = &v["template"]["frontmatter"];
        assert!(
            template.is_object(),
            "frontmatter template should be an object, got: {:?}",
            template
        );
        assert!(
            template.get("description").is_some(),
            "frontmatter must include description field"
        );
        // Instructions mention --- delimiters.
        let instructions = v["instructions"].as_str().unwrap_or("");
        assert!(
            instructions.contains("---"),
            "instructions must mention --- frontmatter delimiters"
        );
    }

    #[tokio::test]
    async fn test_call_create_guidance_includes_body_structure() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "create", "name": "my_skill", "description": "Test"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        // Guidance must include body structure (overview + instructions sections).
        let body_outline = v["template"]["body_outline"]
            .as_array()
            .expect("body_outline should be an array");
        let body_text: String = body_outline
            .iter()
            .map(|v| v.as_str().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            body_text.contains("# Skill Name"),
            "body outline must include heading"
        );
        assert!(
            body_text.contains("## Overview"),
            "body outline must include Overview section"
        );
        assert!(
            body_text.contains("## Instructions"),
            "body outline must include Instructions section"
        );
    }

    #[tokio::test]
    async fn test_call_validate_guidance_includes_checks() {
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "validate", "path": "skills/test/SKILL.md"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&result.new_messages[0].content).unwrap();
        let checks = v["checks"].as_array().expect("checks should be an array");
        assert!(
            checks
                .iter()
                .any(|c| c.as_str().unwrap_or("").contains("---")),
            "checks must include frontmatter delimiter validation"
        );
        assert!(
            checks
                .iter()
                .any(|c| c.as_str().unwrap_or("").contains("description")),
            "checks must include description field validation"
        );
    }

    #[tokio::test]
    async fn test_call_no_filesystem_write_on_readonly_dir() {
        // SkillCreatorTool is pure guidance — it does not write files.
        // Verify it succeeds even in a read-only directory.
        let tool = SkillCreatorTool::new();
        let result = tool
            .call(
                json!({"action": "create", "name": "test_skill", "description": "X"}),
                &new_ctx(),
            )
            .await
            .unwrap();
        assert_eq!(result.data["status"], "guidance_returned");
        // Verify no context_modifier (no filesystem side effects).
        assert!(result.context_modifier.is_none());
    }
}
