//! Fork-mode integration tests for SkillTool.
//!
//! Extracted from `skill_tool.rs` to keep the main file under 500 lines.
//!
//! NOTE: Tests requiring `SpawnController` (a main-crate type) are marked
//! `#[ignore]` until they can be moved to integration tests or the main
//! crate test suite.

#[cfg(test)]
mod tests {
    use super::super::skill_tool::SkillTool;
    use crate::{
        SpawnError, SpawnValidationResult, SpawnValidator, Tool, ToolCallError, ToolContext,
    };
    use closeclaw_config::agents::{ConfigSource, MemoryConfig, ResolvedAgentConfig};
    use closeclaw_gateway::{GatewayConfig, SessionManager};
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use closeclaw_skills::disk::DiskSkillRegistry;
    use std::sync::Arc;
    use tempfile::TempDir;

    /// A pass-through SpawnValidator for inline/agent mode tests.
    struct StubSpawnValidator;

    #[async_trait::async_trait]
    impl SpawnValidator for StubSpawnValidator {
        async fn validate_spawn(
            &self,
            _parent_session_id: &str,
            _target_agent_id: Option<&str>,
        ) -> Result<SpawnValidationResult, SpawnError> {
            Ok(SpawnValidationResult {
                config: ResolvedAgentConfig {
                    id: "stub-agent".to_string(),
                    name: "stub-agent".to_string(),
                    parent_id: None,
                    model: None,
                    workspace: None,
                    agent_dir: None,
                    bootstrap_mode: BootstrapMode::Full,
                    skills: vec![],
                    tools: vec![],
                    disallowed_tools: vec![],
                    subagents: Default::default(),
                    memory: MemoryConfig::default(),
                    source: ConfigSource::Merged,
                },
                effective_max_spawn_depth: 10,
            })
        }
    }

    fn make_session_manager(workspace: Option<std::path::PathBuf>) -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                dm_scope: closeclaw_gateway::DmScope::default(),
                ..Default::default()
            },
            None,
            workspace,
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ))
    }

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
            session_mode: None,
        }
    }

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
            body: String::new(),
        }
    }

    // -----------------------------------------------------------------
    // Inline / Agent mode tests (no SpawnController needed)
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn test_call_inline_mode() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content =
            "---\ndescription: A test skill\n---\n\n# Test Skill\n\nSome skill content here.\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("testskill", vec![], readme_path);
        skill.body = "# Test Skill\n\nSome skill content here.".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);

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
    async fn test_call_agent_mode() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: An agent skill\n---\n\n# Agent Skill\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("agentskill", vec![], readme_path);
        skill.body = "# Agent Skill".to_string();
        skill.manifest.context = SkillContext::Agent {
            agent_id: "my-agent".to_string(),
        };
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);

        let result = tool
            .call(serde_json::json!({"skill_name": "agentskill"}), &new_ctx())
            .await
            .unwrap();

        assert_eq!(result.data["execution_mode"], "agent");
        assert_eq!(result.data["agent_id"], "my-agent");
    }

    #[tokio::test]
    async fn test_call_injects_body_only() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: Some description\ntags:\n  - test\n---\n\n"
            .to_string()
            + "# Actual Skill Body\n\nThis is the real content.\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill("testskill", vec![], readme_path);
        skill.body = "# Actual Skill Body\n\nThis is the real content.".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);

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

    #[tokio::test]
    async fn test_call_normal_with_allowed_tools() {
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        let skill_content = "---\ndescription: Skill with allowed tools\n---\n\n# My Skill\n";
        std::fs::write(&readme_path, skill_content).unwrap();

        let mut skill = make_skill(
            "tooled",
            vec!["ReadTool".into(), "WriteTool".into()],
            readme_path,
        );
        skill.body = "# My Skill".to_string();
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);

        let result = tool
            .call(serde_json::json!({"skill_name": "tooled"}), &new_ctx())
            .await
            .unwrap();

        assert!(result.context_modifier.is_some());
        let cm = result.context_modifier.unwrap();
        assert_eq!(cm.allowed_tools, vec!["ReadTool", "WriteTool"]);
        assert_eq!(result.data["execution_mode"], "inline");
    }

    // -----------------------------------------------------------------
    // Fork-mode tests (require SpawnController from main crate)
    // -----------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires SpawnController from main crate"]
    async fn test_call_fork_mode_spawns_child() {
        // Fork mode requires SpawnController which lives in the main crate.
        // This test should be moved to integration tests or the main crate.
    }

    #[tokio::test]
    #[ignore = "requires SpawnController from main crate"]
    async fn test_call_fork_mode_no_allowed_tools() {
        // Fork mode requires SpawnController which lives in the main crate.
    }

    #[tokio::test]
    async fn test_call_skill_not_found() {
        let registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);
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
        let sm = make_session_manager(None);
        let tool = SkillTool::new(registry, Arc::new(StubSpawnValidator), sm);
        let result = tool.call(serde_json::json!({}), &new_ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolCallError::InvalidArgs(_)));
    }
}
