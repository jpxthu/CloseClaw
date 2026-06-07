//! Fork-mode integration tests for SkillTool.
//!
//! Extracted from `skill_tool.rs` to keep the main file under 500 lines.

#[cfg(test)]
mod tests {
    use super::super::skill_tool::SkillTool;
    use crate::agent::config::SubagentsConfig;
    use crate::agent::spawn::SpawnController;
    use crate::config::agents::{ConfigSource, ResolvedAgentConfig};
    use crate::config::ConfigManager;
    use crate::gateway::{GatewayConfig, SessionManager};
    use crate::llm::session::ConversationSession;
    use crate::session::bootstrap::loader::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;
    use crate::skills::disk::types::{
        DiskSkill, SkillContext, SkillEffort, SkillManifest, SkillSource,
    };
    use crate::skills::disk::DiskSkillRegistry;
    use crate::tools::{Tool, ToolContext};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    fn test_deps() -> (Arc<SpawnController>, Arc<SessionManager>) {
        let tmp = TempDir::new().unwrap();
        let config_manager = Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("ConfigManager::new should succeed"),
        );
        let session_manager = Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                dm_scope: crate::gateway::DmScope::default(),
                ..Default::default()
            },
            None,
            None,
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ));
        let spawn_controller = Arc::new(SpawnController::new(
            config_manager,
            session_manager.clone(),
        ));
        (spawn_controller, session_manager)
    }

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        }
    }

    /// Helper to build a ConfigManager pre-populated with parent + child
    /// agent configs, a SessionManager, and register a parent
    /// ConversationSession.
    async fn setup_fork_env() -> (
        Arc<ConfigManager>,
        Arc<SessionManager>,
        Arc<SpawnController>,
    ) {
        let tmp = TempDir::new().unwrap();
        let config_manager = Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("ConfigManager::new should succeed"),
        );

        // Insert parent agent config with default_child_agent set
        let parent_config = ResolvedAgentConfig {
            id: "parent-agent".to_string(),
            name: "parent-agent".to_string(),
            parent_id: None,
            model: Some("test-model".to_string()),
            workspace: Some(tmp.path().to_path_buf()),
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: SubagentsConfig {
                default_child_agent: Some("child-agent".to_string()),
                require_agent_id: false,
                max_spawn_depth: 3,
                max_children: 16,
                allow_agents: vec!["*".to_string()],
                model: None,
            },
            source: ConfigSource::Merged,
        };
        {
            let mut agents = config_manager.agents.write().unwrap();
            agents.insert("parent-agent".to_string(), parent_config);
        }

        // Insert child/target agent config
        let child_config = ResolvedAgentConfig {
            id: "child-agent".to_string(),
            name: "child-agent".to_string(),
            parent_id: Some("parent-agent".to_string()),
            model: Some("test-model".to_string()),
            workspace: Some(tmp.path().to_path_buf()),
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: SubagentsConfig::default(),
            source: ConfigSource::Merged,
        };
        {
            let mut agents = config_manager.agents.write().unwrap();
            agents.insert("child-agent".to_string(), child_config);
        }

        let session_manager = Arc::new(SessionManager::new(
            &GatewayConfig {
                name: "test".to_string(),
                rate_limit_per_minute: 100,
                max_message_size: 1024,
                dm_scope: crate::gateway::DmScope::default(),
                ..Default::default()
            },
            None,
            Some(tmp.path().to_path_buf()),
            BootstrapMode::Full,
            ReasoningLevel::default(),
        ));

        // Register parent session in conversation_sessions
        let parent_id = "parent-session-fork";
        let cs = ConversationSession::new(
            parent_id.to_string(),
            "test-model".to_string(),
            tmp.path().to_path_buf(),
        );
        let arc = Arc::new(RwLock::new(cs));
        {
            let mut conv = session_manager.conversation_sessions.write().await;
            conv.insert(parent_id.to_string(), arc);
        }
        // Also register in sessions map so depth lookup works
        {
            let mut sessions = session_manager.sessions.write().await;
            sessions.insert(
                parent_id.to_string(),
                crate::gateway::Session {
                    id: parent_id.to_string(),
                    agent_id: "parent-agent".to_string(),
                    channel: "test".to_string(),
                    created_at: chrono::Utc::now().timestamp(),
                    depth: 0,
                },
            );
        }

        let spawn_controller = Arc::new(SpawnController::new(
            config_manager.clone(),
            session_manager.clone(),
        ));

        (config_manager, session_manager, spawn_controller)
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

    #[tokio::test]
    async fn test_call_fork_mode_spawns_child() {
        let (_cm, sm, sc) = setup_fork_env().await;

        // Create a fork skill with body and allowed_tools
        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        std::fs::write(
            &readme_path,
            "---\ndescription: Fork skill\n---\n\n# Fork Skill\n",
        )
        .unwrap();

        let mut skill = make_skill(
            "forkskill",
            vec!["ReadTool".into(), "WriteTool".into()],
            readme_path,
        );
        skill.body = "# Fork Skill\n\nDo the fork thing.".to_string();
        skill.manifest.context = SkillContext::Fork;
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry, sc, sm.clone());

        let ctx = ToolContext {
            agent_id: "parent-agent".to_string(),
            workdir: None,
            session_id: Some("parent-session-fork".to_string()),
            call_id: None,
            session: None,
        };

        let result = tool
            .call(serde_json::json!({"skill_name": "forkskill"}), &ctx)
            .await;

        let result = result.expect("fork mode should succeed");
        assert_eq!(result.data["execution_mode"], "fork");
        assert_eq!(result.data["status"], "spawned");
        assert!(result.data["child_session_id"].is_string());
        let child_id = result.data["child_session_id"].as_str().unwrap();

        // Verify the child session was actually created
        assert!(sm.has_session(child_id).await);
        assert_eq!(sm.get_session_depth(child_id).await, Some(1));

        // Verify the child session has the skill body as its pending task
        let cs = sm
            .get_conversation_session(child_id)
            .await
            .expect("child conversation session should exist");
        let pending = cs.read().await.get_pending_messages();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].content, "# Fork Skill\n\nDo the fork thing.");
    }

    #[tokio::test]
    async fn test_call_fork_mode_no_allowed_tools() {
        let (_cm, sm, sc) = setup_fork_env().await;

        let temp = TempDir::new().unwrap();
        let readme_path = temp.path().join("SKILL.md");
        std::fs::write(&readme_path, "---\ndescription: Simple fork\n---\n").unwrap();

        let mut skill = make_skill("simplefork", vec![], readme_path);
        skill.body = "Simple task".to_string();
        skill.manifest.context = SkillContext::Fork;
        let registry = Arc::new(DiskSkillRegistry::new(vec![skill]));
        let tool = SkillTool::new(registry, sc, sm.clone());

        let ctx = ToolContext {
            agent_id: "parent-agent".to_string(),
            workdir: None,
            session_id: Some("parent-session-fork".to_string()),
            call_id: None,
            session: None,
        };

        let result = tool
            .call(serde_json::json!({"skill_name": "simplefork"}), &ctx)
            .await
            .expect("fork mode should succeed");

        assert_eq!(result.data["execution_mode"], "fork");
        let child_id = result.data["child_session_id"].as_str().unwrap();
        assert!(sm.has_session(child_id).await);

        // context_modifier should be None when no allowed_tools
        assert!(result.context_modifier.is_none());
    }

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
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);

        let result = tool
            .call(serde_json::json!({"skill_name": "testskill"}), &new_ctx())
            .await
            .unwrap();

        assert_eq!(result.data["skill_name"], "testskill");
        assert_eq!(result.data["status"], "loaded");
        assert_eq!(result.data["execution_mode"], "inline");
        assert_eq!(result.new_messages.len(), 1);
        assert!(result.new_messages[0].is_meta);
        // body is used directly (populated by loader)
        assert_eq!(
            result.new_messages[0].content,
            "# Test Skill\n\nSome skill content here."
        );
        // Ensure frontmatter is NOT present in injected content
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
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);

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
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);

        let result = tool
            .call(serde_json::json!({"skill_name": "testskill"}), &new_ctx())
            .await
            .unwrap();

        let content = result.new_messages[0].content.as_str();
        // Must NOT contain frontmatter delimiters
        assert!(
            !content.contains("---"),
            "content should not contain frontmatter delimiters"
        );
        // Must NOT contain YAML field names from frontmatter
        assert!(
            !content.contains("description:"),
            "content should not contain YAML fields"
        );
        assert!(
            !content.contains("tags:"),
            "content should not contain YAML fields"
        );
        // Must contain the body text
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
        let (sc, sm) = test_deps();
        let tool = SkillTool::new(registry, sc, sm);

        let result = tool
            .call(serde_json::json!({"skill_name": "tooled"}), &new_ctx())
            .await
            .unwrap();

        assert!(result.context_modifier.is_some());
        let cm = result.context_modifier.unwrap();
        assert_eq!(cm.allowed_tools, vec!["ReadTool", "WriteTool"]);
        assert_eq!(result.data["execution_mode"], "inline");
    }
}
