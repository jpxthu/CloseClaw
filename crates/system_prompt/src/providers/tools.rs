//! Provider for the Tools section of the system prompt.
//!
//! Delegates to the existing `build_tools_section` logic and wraps the
//! result as a [`PromptFragment`].

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_tools::{ToolContext, ToolRegistry};

use crate::fragment::{FragmentContext, PromptFragment, PromptFragmentProvider, SectionType};

/// Provider that contributes the tool listing to the system prompt.
///
/// Holds references to the [`ToolRegistry`] and agent-level tool
/// configuration. When the registry is empty or produces no content,
/// [`generate`](Self::generate) returns `None`.
pub struct ToolsFragmentProvider {
    registry: Arc<ToolRegistry>,
    /// Agent-level tool whitelist (`tools` field in agent config).
    agent_tools: Option<Vec<String>>,
    /// Agent-level tool blacklist (`disallowedTools` field in agent config).
    agent_disallowed_tools: Option<Vec<String>>,
}

impl ToolsFragmentProvider {
    pub fn new(
        registry: Arc<ToolRegistry>,
        agent_tools: Option<Vec<String>>,
        agent_disallowed_tools: Option<Vec<String>>,
    ) -> Self {
        Self {
            registry,
            agent_tools,
            agent_disallowed_tools,
        }
    }

    /// Build a [`ToolContext`] from a [`FragmentContext`].
    fn tool_context(ctx: &FragmentContext) -> ToolContext {
        let workdir = ctx.workdir.as_ref().map(|p| {
            let path_str = p.to_string_lossy().to_string();
            closeclaw_tools::build_workdir_context(&path_str)
        });
        ToolContext {
            agent_id: ctx.agent_id.clone().unwrap_or_default(),
            workdir,
            session_id: None,
            call_id: None,
            session: None,
        }
    }
}

#[async_trait]
impl PromptFragmentProvider for ToolsFragmentProvider {
    fn name(&self) -> &str {
        "tools"
    }

    fn priority(&self) -> u32 {
        2
    }

    async fn generate(&self, ctx: &FragmentContext) -> Option<PromptFragment> {
        let tool_ctx = Self::tool_context(ctx);
        let section = crate::tools_section::build_tools_section(
            &self.registry,
            &tool_ctx,
            self.agent_tools.clone(),
            self.agent_disallowed_tools.clone(),
        )
        .await;

        // Extract content from the rendered Section.
        let content = match section {
            crate::sections::Section::ToolsSection(c) => c,
            _ => return None,
        };

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Tools".to_string(),
            section_type: SectionType::Tools,
            content,
        })
    }

    /// Registry-backed — no file mtime to key on.
    fn cache_key(&self, _ctx: &FragmentContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_provider_name_and_priority() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        assert_eq!(provider.name(), "tools");
        assert_eq!(provider.priority(), 2);
    }

    #[test]
    fn test_cache_key_always_none() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        let ctx = FragmentContext::default();
        assert!(provider.cache_key(&ctx).is_none());
    }

    #[tokio::test]
    async fn test_generate_empty_registry_returns_none() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        let ctx = FragmentContext::default();
        // Empty registry → no tools → content is empty → None
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_with_tools() {
        let registry = Arc::new(ToolRegistry::new());
        let disk_registry = Arc::new(closeclaw_skills::DiskSkillRegistry::new(vec![]));

        // Register tools via the new Registrar pattern.
        let permission_engine = Arc::new(
            closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
                closeclaw_permission::rules::RuleSetBuilder::new()
                    .build()
                    .unwrap(),
            ),
        );
        let tmp = tempfile::tempdir().unwrap();
        let cfg_mgr =
            Arc::new(closeclaw_config::ConfigManager::new(tmp.path().to_path_buf()).unwrap());
        let cfg = closeclaw_gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            dm_scope: closeclaw_gateway::DmScope::PerChannelPeer,
            ..Default::default()
        };
        let session_manager = Arc::new(closeclaw_gateway::SessionManager::new(
            &cfg,
            None,
            None,
            closeclaw_session::bootstrap::BootstrapMode::Minimal,
            closeclaw_session::persistence::ReasoningLevel::default(),
        ));
        let spawn_controller = Arc::new(closeclaw_gateway::SpawnController::new(
            Arc::clone(&cfg_mgr),
            Arc::clone(&session_manager),
            permission_engine.clone(),
        ));
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());

        let task_manager = Arc::new(closeclaw_tasks::BackgroundTaskManager::new());
        let registrars: Vec<Box<dyn closeclaw_tools::ToolRegistrar>> = vec![
            Box::new(closeclaw_tools::CoreToolsRegistrar::new(
                permission_engine.clone(),
                task_manager as Arc<dyn closeclaw_common::TaskManager>,
                session_manager.clone(),
                cfg_mgr.clone(),
            )),
            Box::new(closeclaw_tools::SessionToolsRegistrar::new(
                spawn_controller.clone() as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager.clone(),
                agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
                permission_engine,
            )),
            Box::new(closeclaw_tools::SkillsToolsRegistrar::new(
                disk_registry,
                spawn_controller as Arc<dyn closeclaw_tools::SpawnValidator>,
                session_manager,
            )),
        ];
        registry.register_all(registrars).await.unwrap();

        let provider = ToolsFragmentProvider::new(registry, None, None);
        let ctx = FragmentContext::default();
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.section_type, SectionType::Tools);
        assert!(frag.content.contains("file_ops"));
    }
}
