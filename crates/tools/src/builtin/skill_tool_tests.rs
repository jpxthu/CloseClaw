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

        let skill = make_skill("testskill", readme_path);
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

        let skill = make_skill("testskill", readme_path);
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
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        std::fs::write(
            &readme_path,
            "---\ndescription: Shared skill\n---\n\nDisk body.\n",
        )
        .unwrap();
        let disk_skill = make_skill("shared", readme_path);
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

    // -----------------------------------------------------------------
    // Variable substitution integration tests (via call())
    // -----------------------------------------------------------------

    fn make_skill_with_body(
        name: &str,
        body: &str,
        skill_dir: std::path::PathBuf,
    ) -> (DiskSkill, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let content = format!(
            "---\ndescription: A test skill named {}\n---\n\n{}\n",
            name, body
        );
        std::fs::write(&readme_path, content).unwrap();
        (
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
                skill_dir,
            },
            temp,
        )
    }

    fn new_ctx_with_session(session_id: Option<String>) -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id,
            call_id: None,
            session: None,
            session_mode: None,
            manual_background_signal: None,
        }
    }

    #[tokio::test]
    async fn test_call_substitutes_skill_dir() {
        let (skill, _temp) = make_skill_with_body(
            "test",
            "Read files in ${SKILL_DIR}",
            std::path::PathBuf::from("/home/user/.closeclaw/skills/my-skill"),
        );
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(disk, Arc::new(BuiltinSkillRegistry::new()));
        let result = tool
            .call(serde_json::json!({"skill_name": "test"}), &new_ctx())
            .await
            .unwrap();
        assert_eq!(
            result.new_messages[0].content,
            "Read files in /home/user/.closeclaw/skills/my-skill"
        );
    }

    #[tokio::test]
    async fn test_call_substitutes_session_id() {
        let (skill, _temp) = make_skill_with_body(
            "test",
            "Session: ${SESSION_ID}",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(disk, Arc::new(BuiltinSkillRegistry::new()));
        let ctx = new_ctx_with_session(Some("sess-abc-123".to_string()));
        let result = tool
            .call(serde_json::json!({"skill_name": "test"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.new_messages[0].content, "Session: sess-abc-123");
    }

    #[tokio::test]
    async fn test_call_preserves_unknown_variables() {
        let (skill, _temp) = make_skill_with_body(
            "test",
            "Hello ${UNKNOWN_VAR}",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(disk, Arc::new(BuiltinSkillRegistry::new()));
        let result = tool
            .call(serde_json::json!({"skill_name": "test"}), &new_ctx())
            .await
            .unwrap();
        assert_eq!(result.new_messages[0].content, "Hello ${UNKNOWN_VAR}");
    }

    #[tokio::test]
    async fn test_call_substitute_mixed_known_and_unknown() {
        let (skill, _temp) = make_skill_with_body(
            "test",
            "Dir: ${SKILL_DIR}, Session: ${SESSION_ID}, Unknown: ${FOO}",
            std::path::PathBuf::from("/tmp/my-skill"),
        );
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(disk, Arc::new(BuiltinSkillRegistry::new()));
        let ctx = new_ctx_with_session(Some("s-999".to_string()));
        let result = tool
            .call(serde_json::json!({"skill_name": "test"}), &ctx)
            .await
            .unwrap();
        assert_eq!(
            result.new_messages[0].content,
            "Dir: /tmp/my-skill, Session: s-999, Unknown: ${FOO}"
        );
    }

    #[tokio::test]
    async fn test_call_no_context_modifier_for_disk_skill() {
        let (skill, _temp) = make_skill_with_body(
            "test",
            "No modifier",
            std::path::PathBuf::from("/tmp/skill"),
        );
        let disk = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(disk, Arc::new(BuiltinSkillRegistry::new()));
        let result = tool
            .call(serde_json::json!({"skill_name": "test"}), &new_ctx())
            .await
            .unwrap();
        assert!(result.context_modifier.is_none());
        assert!(result.new_messages[0].is_meta);
    }
}
