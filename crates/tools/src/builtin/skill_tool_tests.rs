//! Tests for SkillTool — inline-mode and builtin fallback.

#[cfg(test)]
mod tests {
    use super::super::skill_tool::SkillTool;
    use crate::{Tool, ToolCallError, ToolContext};
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use closeclaw_skills::disk::DiskSkillRegistry;
    use closeclaw_skills::BuiltinSkillRegistry;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        }
    }

    fn make_skill(name: &str, readme_path: std::path::PathBuf) -> DiskSkill {
        DiskSkill {
            source: SkillSource::Bundled,
            manifest: SkillManifest {
                name: name.into(),
                description: format!("A test skill named {}", name),
                when_to_use: String::new(),
                context: SkillContext::Inline,
                effort: SkillEffort::Small,
                paths: vec![],
                user_invocable: false,
            },
            readme_path,
            skill_dir: std::path::PathBuf::new(),
            body: String::new(),
        }
    }

    // -----------------------------------------------------------------
    // Inline-mode tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_inline_mode() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content =
            "---\ndescription: A test skill\n---\n\n# Test Skill\n\nSome skill content here.\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("testskill", readme_path);
        skill.body = "# Test Skill\n\nSome skill content here.".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry, Arc::new(BuiltinSkillRegistry::new()));

        let result = tool
            .call(serde_json::json!({"skill_name": "testskill"}), &new_ctx())
            .await
            .unwrap();

        assert_eq!(result.data["skill_name"], "testskill");
        assert_eq!(result.data["status"], "loaded");
        assert_eq!(result.data["execution_mode"], "inline");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        assert_eq!(
            result.new_messages[0].content,
            "# Test Skill\n\nSome skill content here."
        );
        assert!(!result.new_messages[0].content.contains("---"));
        assert!(result.context_modifier.is_none());
    }

    #[tokio::test]
    async fn test_call_injects_body_only() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: Some description\ntags:\n  - test\n---\n\n"
            .to_string()
            + "# Actual Skill Body\n\nThis is the real content.\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("testskill", readme_path);
        skill.body = "# Actual Skill Body\n\nThis is the real content.".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry, Arc::new(BuiltinSkillRegistry::new()));

        let result = tool
            .call(serde_json::json!({"skill_name": "testskill"}), &new_ctx())
            .await
            .unwrap();

        let content = result.new_messages[0].content.as_str();
        assert!(
            !content.contains("---"),
            "content should not contain frontmatter delimiters"
        );
        assert!(
            !content.contains("description:"),
            "content should not contain YAML fields"
        );
        assert!(
            !content.contains("tags:"),
            "content should not contain YAML fields"
        );
        assert!(
            content.contains("# Actual Skill Body"),
            "content should contain body heading"
        );
        assert!(
            content.contains("This is the real content."),
            "content should contain body text"
        );
    }

    // -----------------------------------------------------------------
    // Error-path tests
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry, Arc::new(BuiltinSkillRegistry::new()));
        let result = tool
            .call(serde_json::json!({"skill_name": "nonexistent"}), &new_ctx())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_call_missing_skill_name() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let tool = SkillTool::new(registry, Arc::new(BuiltinSkillRegistry::new()));
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }

    // -----------------------------------------------------------------
    // Builtin fallback tests
    // -----------------------------------------------------------------

    struct MockBuiltinSkill(String);

    #[async_trait::async_trait]
    impl closeclaw_skills::Skill for MockBuiltinSkill {
        fn manifest(&self) -> closeclaw_skills::SkillManifest {
            closeclaw_skills::SkillManifest {
                name: self.0.clone(),
                version: "1.0".into(),
                description: "mock builtin".into(),
                author: None,
                dependencies: vec![],
            }
        }
        fn methods(&self) -> Vec<&str> {
            vec!["invoke"]
        }
        async fn execute(
            &self,
            _method: &str,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, closeclaw_skills::SkillError> {
            Ok(serde_json::json!({"output": "builtin result"}))
        }
    }

    #[tokio::test]
    async fn test_call_builtin_skill_fallback() {
        let disk = Arc::new(DiskSkillRegistry::new(vec![]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(MockBuiltinSkill("my_builtin".into())))
            .await;
        let tool = SkillTool::new(disk, builtin);
        let result = tool
            .call(serde_json::json!({"skill_name": "my_builtin"}), &new_ctx())
            .await
            .unwrap();
        assert_eq!(result.data["skill_name"], "my_builtin");
        assert_eq!(result.data["status"], "loaded");
        assert_eq!(result.data["execution_mode"], "inline");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        assert!(result.new_messages[0].content.contains("builtin result"));
        assert!(result.context_modifier.is_none());
    }

    #[tokio::test]
    async fn test_call_disk_priority_over_builtin() {
        let disk_skill = make_skill("shared", std::path::PathBuf::from("/tmp/test"));
        let disk = Arc::new(DiskSkillRegistry::new(vec![disk_skill]));
        let builtin = Arc::new(BuiltinSkillRegistry::new());
        builtin
            .register(Arc::new(MockBuiltinSkill("shared".into())))
            .await;
        let tool = SkillTool::new(disk, builtin);
        let result = tool
            .call(serde_json::json!({"skill_name": "shared"}), &new_ctx())
            .await
            .unwrap();
        assert_eq!(result.data["execution_mode"], "inline");
        assert!(result.context_modifier.is_none());
    }
}
