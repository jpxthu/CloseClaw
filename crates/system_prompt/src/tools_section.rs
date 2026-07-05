//! Tools section builder for the system prompt.
//!
//! Owns the `build_tools_section` function and its tests. Extracted from
//! `builder.rs` to keep that file under the 500-line limit.

use crate::sections::Section;
use closeclaw_tools::{PromptGenerationContext, ToolContext, ToolRegistry};

/// Task writing guidance appended to the tools section when spawn is available.
/// Source: docs/design/agent/agent-spawn.md §父 Agent 的 Task 编写指引
const TASK_WRITING_GUIDANCE: &str = concat!(
    "\n\n## Task Writing Guidance for Spawning Sub-Agents\n\n\n",
    "When spawning a sub-agent, write the task as you would brief a smart colleague ",
    "who just walked into the room \u{2014} explain what you need done and why.\n\n",
    "- Do NOT push judgment calls onto the sub-agent. The parent agent should ",
    "complete understanding and decision-making; the sub-agent executes.\n",
    "- Use fork mode when the sub-agent needs full context of the ongoing conversation. ",
    "Use normal spawn for independent, self-contained tasks."
);

/// Build the Tools section content from a registry.
///
/// The registry's `build_tools_section` requires a [`PromptGenerationContext`]
/// (which carries the list of available tool names, the agent id, and the
/// workdir). We acquire that list via a single short-lived lock on
/// `list_descriptors`, release it, and then call the registry's
/// `build_tools_section` with the freshly-built context. This keeps locks
/// non-overlapping.
pub async fn build_tools_section(
    registry: &ToolRegistry,
    ctx: &ToolContext,
    agent_tools: Option<Vec<String>>,
    agent_disallowed_tools: Option<Vec<String>>,
) -> Section {
    // 1. Independent lock: get available tool names, then drop the lock.
    let descriptors = registry.list_descriptors(ctx).await;
    let available_tool_names: Vec<String> = descriptors.into_iter().map(|d| d.name).collect();

    // 2. Resolve agent-level tool filtering.
    //    Priority: explicit parameters > AgentRegistry query.
    let (tools, disallowed_tools) = if agent_tools.is_some() || agent_disallowed_tools.is_some() {
        (agent_tools, agent_disallowed_tools)
    } else {
        // Query AgentRegistry directly (design-doc query path).
        registry.query_agent_tools_config(&ctx.agent_id).await
    };

    // 3. Build the prompt-generation context from the names + the existing
    //    execution context, including agent-level tool filtering.
    let prompt_ctx = PromptGenerationContext {
        agent_id: ctx.agent_id.clone(),
        workdir: ctx.workdir.clone(),
        available_tool_names,
        tools,
        disallowed_tools,
    };

    // 4. Acquire the registry lock again and render the section.
    let content = registry.build_tools_section(&prompt_ctx).await;

    // 5. If the spawn tool is available, append task writing guidance.
    let content = if prompt_ctx
        .available_tool_names
        .iter()
        .any(|n| n == "sessions_spawn")
    {
        let guidance = TASK_WRITING_GUIDANCE;
        format!("{}\n{}", content, guidance)
    } else {
        content
    };

    Section::ToolsSection(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_agent::registry::AgentRegistry;
    use closeclaw_config::ConfigManager;
    use closeclaw_gateway::SpawnController;
    use closeclaw_gateway::{GatewayConfig, SessionManager};
    use closeclaw_permission::engine::engine_eval::PermissionEngine;
    use closeclaw_permission::rules::RuleSetBuilder;
    use closeclaw_session::bootstrap::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;
    use closeclaw_skills::DiskSkillRegistry;
    use closeclaw_tasks::BackgroundTaskManager;
    use closeclaw_tools::{
        CoreToolsRegistrar, SessionToolsRegistrar, SkillsToolsRegistrar, ToolRegistrar,
    };
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_permission_engine() -> Arc<PermissionEngine> {
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        ))
    }

    /// Build a minimal SpawnController + SessionManager pair for tests
    /// that only need to exercise the tool-registration path.
    fn test_spawn_deps() -> (
        Arc<SpawnController>,
        Arc<SessionManager>,
        Arc<ConfigManager>,
        Arc<AgentRegistry>,
    ) {
        let tmp = TempDir::new().expect("tempdir for test");
        let cfg_mgr = Arc::new(
            ConfigManager::new(tmp.path().to_path_buf())
                .expect("failed to create ConfigManager for test"),
        );
        let cfg = GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: closeclaw_gateway::DmScope::PerChannelPeer,
            ..Default::default()
        };
        let session_manager = Arc::new(SessionManager::new(
            &cfg,
            None,
            None,
            BootstrapMode::Minimal,
            ReasoningLevel::default(),
        ));
        let spawn_controller = Arc::new(SpawnController::new(
            Arc::clone(&cfg_mgr),
            Arc::clone(&session_manager),
            Arc::new(PermissionEngine::new_with_default_data_root(
                RuleSetBuilder::new().build().unwrap(),
            )),
        ));
        let agent_registry = Arc::new(AgentRegistry::new());
        (spawn_controller, session_manager, cfg_mgr, agent_registry)
    }

    fn make_registrars(
        disk_registry: Arc<DiskSkillRegistry>,
        permission_engine: Arc<PermissionEngine>,
        spawn_controller: Arc<SpawnController>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        agent_registry: Arc<AgentRegistry>,
    ) -> Vec<Box<dyn ToolRegistrar>> {
        let task_manager = Arc::new(BackgroundTaskManager::new());
        vec![
            Box::new(CoreToolsRegistrar::new(
                permission_engine.clone(),
                task_manager as Arc<dyn closeclaw_tasks::TaskManager>,
                session_manager.clone(),
                config_manager,
            )),
            Box::new(SessionToolsRegistrar::new(
                spawn_controller.clone() as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager.clone(),
                agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
                permission_engine,
            )),
            Box::new(SkillsToolsRegistrar::new(
                disk_registry,
                spawn_controller as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager,
            )),
        ]
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_tools_section() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        match section {
            Section::ToolsSection(_) => {}
            _ => panic!("expected ToolsSection, got {:?}", section),
        }
    }

    #[tokio::test]
    async fn test_build_tools_section_contains_group_headers() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("file_ops"),
            "missing file_ops group: {}",
            content
        );
        assert!(content.contains("meta"), "missing meta group: {}", content);
    }

    #[tokio::test]
    async fn test_build_tools_section_contains_tool_names() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        for name in &[
            "Read",
            "Write",
            "Edit",
            "Grep",
            "Ls",
            "ToolSearch",
            "PermissionQuery",
        ] {
            assert!(
                content.contains(name),
                "tool {} not found in: {}",
                name,
                content
            );
        }
    }

    #[tokio::test]
    async fn test_build_tools_section_respects_max_length() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.chars().count() <= 15000,
            "section too long: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_build_tools_section_empty_registry() {
        let registry = ToolRegistry::new();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.is_empty(),
            "expected empty content, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_task_writing_guidance_when_spawn_available() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        registry
            .register_all(make_registrars(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ))
            .await
            .unwrap();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.contains("smart colleague"),
            "missing 'smart colleague' in guidance: {}",
            &content[content.len().min(content.len().saturating_sub(500))..]
        );
        assert!(
            content.contains("judgment calls"),
            "missing 'judgment calls' in guidance"
        );
        assert!(
            content.contains("fork mode"),
            "missing 'fork mode' in guidance"
        );
    }

    #[tokio::test]
    async fn test_task_writing_guidance_absent_when_spawn_unavailable() {
        // Empty registry → sessions_spawn is not in available_tool_names
        let registry = ToolRegistry::new();
        let ctx = closeclaw_tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let section = build_tools_section(&registry, &ctx, None, None).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            !content.contains("smart colleague"),
            "task writing guidance should NOT appear without sessions_spawn, got: {}",
            content
        );
        assert!(
            !content.contains("judgment calls"),
            "task writing guidance should NOT appear without sessions_spawn"
        );
        assert!(
            !content.contains("fork mode"),
            "task writing guidance should NOT appear without sessions_spawn"
        );
    }
}
