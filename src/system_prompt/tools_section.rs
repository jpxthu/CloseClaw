//! Tools section builder for the system prompt.
//!
//! Owns the `build_tools_section` function and its tests. Extracted from
//! `builder.rs` to keep that file under the 500-line limit.

use super::sections::Section;
use crate::tools::{PromptGenerationContext, ToolContext, ToolRegistry};

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

    // 2. Build the prompt-generation context from the names + the existing
    //    execution context, including agent-level tool filtering.
    let prompt_ctx = PromptGenerationContext {
        agent_id: ctx.agent_id.clone(),
        workdir: ctx.workdir.clone(),
        available_tool_names,
        tools: agent_tools,
        disallowed_tools: agent_disallowed_tools,
    };

    // 3. Acquire the registry lock again and render the section.
    let content = registry.build_tools_section(&prompt_ctx).await;
    Section::ToolsSection(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::spawn::SpawnController;
    use crate::config::ConfigManager;
    use crate::gateway::{GatewayConfig, SessionManager};
    use crate::permission::engine::engine_eval::PermissionEngine;
    use crate::permission::rules::RuleSetBuilder;
    use crate::session::bootstrap::BootstrapMode;
    use crate::session::persistence::ReasoningLevel;
    use crate::skills::DiskSkillRegistry;
    use crate::tools::builtin::{register_builtin_tools, BuiltinToolContext};
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
        Arc<crate::agent::registry::AgentRegistry>,
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
            dm_scope: crate::gateway::DmScope::PerChannelPeer,
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
        ));
        let agent_registry = Arc::new(crate::agent::registry::AgentRegistry::new(30));
        (spawn_controller, session_manager, cfg_mgr, agent_registry)
    }

    fn make_builtin_ctx(
        disk_registry: Arc<DiskSkillRegistry>,
        permission_engine: Arc<PermissionEngine>,
        spawn_controller: Arc<SpawnController>,
        session_manager: Arc<SessionManager>,
        config_manager: Arc<ConfigManager>,
        agent_registry: Arc<crate::agent::registry::AgentRegistry>,
    ) -> Arc<BuiltinToolContext> {
        Arc::new(BuiltinToolContext {
            config_manager,
            agent_registry,
            disk_registry,
            permission_engine,
            spawn_controller,
            session_manager,
        })
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_tools_section() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        let (spawn_controller, session_manager, config_manager, agent_registry) = test_spawn_deps();
        register_builtin_tools(
            &registry,
            make_builtin_ctx(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ),
        )
        .await;
        let ctx = crate::tools::ToolContext {
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
        register_builtin_tools(
            &registry,
            make_builtin_ctx(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ),
        )
        .await;
        let ctx = crate::tools::ToolContext {
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
        register_builtin_tools(
            &registry,
            make_builtin_ctx(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ),
        )
        .await;
        let ctx = crate::tools::ToolContext {
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
        register_builtin_tools(
            &registry,
            make_builtin_ctx(
                disk_registry,
                test_permission_engine(),
                spawn_controller,
                session_manager,
                config_manager,
                agent_registry,
            ),
        )
        .await;
        let ctx = crate::tools::ToolContext {
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
        let ctx = crate::tools::ToolContext {
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
}
