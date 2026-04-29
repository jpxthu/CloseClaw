//! Built-in tools — skill creator (stub Tool implementation).
//!
//! This is a placeholder stub. Full implementation tracked in issue #282.

use crate::tools::{Tool, ToolContext, ToolFlags};

use serde_json::Value;

// ---------------------------------------------------------------------------
// SkillCreatorTool
// ---------------------------------------------------------------------------

/// Stub tool for authoring and editing agent skills.
///
/// Real implementation tracked in issue #282.
pub struct SkillCreatorTool;

impl Default for SkillCreatorTool {
    fn default() -> Self {
        Self
    }
}

impl SkillCreatorTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for SkillCreatorTool {
    fn name(&self) -> &str {
        "SkillCreator"
    }

    fn group(&self) -> &str {
        "skill_creator"
    }

    fn summary(&self) -> String {
        "Create or improve agent skills".to_string()
    }

    fn detail(&self) -> String {
        "Stub — real implementation tracked in issue #282. \
         This tool creates, edits, improves, or audits AgentSkills. \
         Use when creating a new skill from scratch or when asked to improve, \
         review, tidy up, or clean up an existing skill or SKILL.md file."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: create, edit, improve, review, or audit",
                    "enum": ["create", "edit", "improve", "review", "audit"]
                },
                "skill_name": {
                    "type": "string",
                    "description": "Name of the skill to create or modify"
                },
                "description": {
                    "type": "string",
                    "description": "Natural language description of the skill purpose"
                }
            },
            "required": ["action", "skill_name"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_destructive: true,
            is_deferred_by_default: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        }
    }

    #[test]
    fn test_skill_creator_name() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.name(), "SkillCreator");
    }

    #[test]
    fn test_skill_creator_group() {
        let tool = SkillCreatorTool::new();
        assert_eq!(tool.group(), "skill_creator");
    }

    #[test]
    fn test_skill_creator_summary_len() {
        let tool = SkillCreatorTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_skill_creator_flags_deferred() {
        let tool = SkillCreatorTool::new();
        assert!(tool.flags().is_deferred_by_default);
    }

    #[test]
    fn test_skill_creator_flags_destructive() {
        let tool = SkillCreatorTool::new();
        assert!(tool.flags().is_destructive);
    }

    #[test]
    fn test_skill_creator_schema_has_action() {
        let tool = SkillCreatorTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("action"));
        assert!(props.contains_key("skill_name"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("action")));
        assert!(required.contains(&serde_json::json!("skill_name")));
    }

    #[test]
    fn test_skill_creator_detail_mentions_282() {
        let tool = SkillCreatorTool::new();
        assert!(tool.detail().contains("#282"));
    }
}
