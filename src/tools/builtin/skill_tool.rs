//! Built-in tool — SkillTool
//!
//! Invokes a disk-based skill by looking it up in the [`DiskSkillRegistry`],
//! reading its SKILL.md file, and returning the content as a meta message
//! to be injected into the agent context.

use crate::skills::disk::DiskSkillRegistry;
use crate::tools::{
    ContextModifier, Tool, ToolCallError, ToolContext, ToolFlags, ToolMessage, ToolResult,
};

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SkillTool
// ---------------------------------------------------------------------------

/// Tool that loads and executes a disk-based skill.
///
/// When called, `SkillTool` looks up the named skill in the
/// [`DiskSkillRegistry`], reads its SKILL.md file, and injects the
/// content as a meta message into the agent context.
pub struct SkillTool {
    registry: Arc<DiskSkillRegistry>,
}

impl SkillTool {
    /// Creates a new `SkillTool` backed by the given registry.
    pub fn new(registry: Arc<DiskSkillRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "SkillTool"
    }

    fn group(&self) -> &str {
        "skills"
    }

    fn summary(&self) -> String {
        "Load and execute a disk-based skill".to_string()
    }

    fn detail(&self) -> String {
        "Loads a skill definition from the disk-based skill registry and \
         makes it available to the agent. Call this tool with `skill_name` \
         (required) to retrieve the skill's SKILL.md content, which will be \
         injected as a meta message. The `args` field (optional) can pass \
         additional context to the skill."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill_name": {
                    "type": "string",
                    "description": "Name of the skill to load (e.g. 'clawhub')"
                },
                "args": {
                    "type": "object",
                    "description": "Optional arguments to pass to the skill"
                }
            },
            "required": ["skill_name"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_deferred_by_default: false,
            ..Default::default()
        }
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        // Extract skill_name from args
        let skill_name = args
            .get("skill_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolCallError::InvalidArgs("skill_name is required".to_string()))?;

        // Look up the skill in the registry
        let skill = self
            .registry
            .get(skill_name)
            .ok_or_else(|| ToolCallError::NotFound(skill_name.to_string()))?;

        // Read the SKILL.md file
        let readme_content = tokio::fs::read_to_string(&skill.readme_path)
            .await
            .map_err(|e| {
                ToolCallError::ExecutionFailed(format!(
                    "failed to read {}: {}",
                    skill.readme_path.display(),
                    e
                ))
            })?;

        // Build context_modifier from manifest.allowed_tools
        let context_modifier = if skill.manifest.allowed_tools.is_empty() {
            None
        } else {
            Some(ContextModifier {
                allowed_tools: skill.manifest.allowed_tools.clone(),
            })
        };

        Ok(ToolResult {
            data: serde_json::json!({
                "skill_name": skill_name,
                "status": "loaded"
            }),
            new_messages: vec![ToolMessage {
                content: readme_content,
                is_meta: true,
            }],
            context_modifier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_skill(
        name: &str,
        allowed_tools: Vec<String>,
        readme_path: std::path::PathBuf,
    ) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("A test skill named {}", name),
                allowed_tools,
                when_to_use: String::new(),
                context: SkillContext::Inline,
                agent: String::new(),
                agent_id: String::new(),
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path,
            skill_dir: std::path::PathBuf::new(),
        }
    }

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
        }
    }

    // -----------------------------------------------------------------
    // Metadata tests
    // -----------------------------------------------------------------

    #[test]
    fn test_skill_tool_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        assert_eq!(tool.name(), "SkillTool");
    }

    #[test]
    fn test_skill_tool_group() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        assert_eq!(tool.group(), "skills");
    }

    #[test]
    fn test_skill_tool_summary_length() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let summary = tool.summary();
        assert!(
            summary.len() <= 50,
            "summary '{}' exceeds 50 chars",
            summary
        );
    }

    #[test]
    fn test_skill_tool_flags_is_deferred_false() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let flags = tool.flags();
        assert!(!flags.is_deferred_by_default);
    }

    #[test]
    fn test_skill_tool_input_schema_contains_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let schema = tool.input_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("skill_name"));
        assert!(props.contains_key("args"));
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("skill_name")));
    }

    // -----------------------------------------------------------------
    // call() — error cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let result = tool
            .call(serde_json::json!({"skill_name": "nonexistent"}), &new_ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::NotFound(_)));
        let ToolCallError::NotFound(name) = err else {
            unreachable!()
        };
        assert_eq!(name, "nonexistent");
    }

    #[tokio::test]
    async fn test_call_missing_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn test_call_skill_name_wrong_type() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry);
        let result = tool
            .call(serde_json::json!({"skill_name": 123}), &new_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolCallError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn test_call_readme_file_not_found() {
        let skill = make_skill(
            "orphan",
            vec![],
            std::path::PathBuf::from("/nonexistent/path/SKILL.md"),
        );
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry);
        let result = tool
            .call(serde_json::json!({"skill_name": "orphan"}), &new_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(result, Err(ToolCallError::ExecutionFailed(_))));
    }

    // -----------------------------------------------------------------
    // call() — normal cases
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_normal_with_empty_allowed_tools() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content =
            "---\ndescription: A test skill\n---\n\n# Test Skill\n\nSome skill content here.\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let skill = make_skill("testskill", vec![], readme_path);
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry);

        let result = tool
            .call(serde_json::json!({"skill_name": "testskill"}), &new_ctx())
            .await
            .unwrap();

        assert_eq!(result.data["skill_name"], "testskill");
        assert_eq!(result.data["status"], "loaded");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        assert_eq!(result.new_messages[0].content, skill_content);
        assert!(result.context_modifier.is_none());
    }

    #[tokio::test]
    async fn test_call_normal_with_allowed_tools() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: Skill with allowed tools\n---\n\n# My Skill\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let skill = make_skill(
            "tooled",
            vec!["ReadTool".into(), "WriteTool".into()],
            readme_path,
        );
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry);

        let result = tool
            .call(serde_json::json!({"skill_name": "tooled"}), &new_ctx())
            .await
            .unwrap();

        assert!(result.context_modifier.is_some());
        let cm = result.context_modifier.unwrap();
        assert_eq!(cm.allowed_tools, vec!["ReadTool", "WriteTool"]);
    }
}
