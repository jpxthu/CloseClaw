//! Provider for the Tools section of the system prompt.
//!
//! Delegates to the existing `build_tools_section` logic and wraps the
//! result as a [`PromptFragment`].

use std::sync::Arc;

use crate::{ToolContext, ToolRegistry};
use async_trait::async_trait;
use closeclaw_common::fragment::{
    FragmentContext, PromptFragment, PromptFragmentProvider, SectionType,
};
use closeclaw_common::SessionMode;

use crate::build_tools_section::{build_tools_section, ToolsSectionParams};

/// Provider that contributes the tool listing to the system prompt.
///
/// Holds references to the [`ToolRegistry`] and an optional
/// [`AgentToolsConfigQuery`] for runtime agent-level tool filtering.
/// When the registry is empty or produces no content,
/// [`generate`](Self::generate) returns `None`.
pub struct ToolsFragmentProvider {
    registry: Arc<ToolRegistry>,
    /// Runtime query for agent-level tool configuration.
    /// When `Some`, the provider queries per-agent tool white/blacklists
    /// at generation time rather than using hardcoded values.
    tools_config_query: Option<Arc<dyn closeclaw_common::AgentToolsConfigQuery>>,
    /// Session mode for mode-aware tool filtering.
    session_mode: Option<SessionMode>,
    /// Agent role (human-readable name/purpose) from agent config.
    agent_role: Option<String>,
    /// Agent type (e.g. root agent vs spawned child) from agent config.
    agent_type: Option<String>,
}

impl ToolsFragmentProvider {
    pub fn new(
        registry: Arc<ToolRegistry>,
        tools_config_query: Option<Arc<dyn closeclaw_common::AgentToolsConfigQuery>>,
        session_mode: Option<SessionMode>,
    ) -> Self {
        Self {
            registry,
            tools_config_query,
            session_mode,
            agent_role: None,
            agent_type: None,
        }
    }

    /// Set the agent role for prompt generation.
    pub fn with_agent_role(mut self, role: String) -> Self {
        self.agent_role = Some(role);
        self
    }

    /// Set the agent type for prompt generation.
    pub fn with_agent_type(mut self, agent_type: String) -> Self {
        self.agent_type = Some(agent_type);
        self
    }

    /// Build a [`ToolContext`] from a [`FragmentContext`].
    fn tool_context(ctx: &FragmentContext, session_mode: Option<SessionMode>) -> ToolContext {
        let path_str = ctx.bootstrap_dir.clone();
        let workdir = Some(crate::build_workdir_context(&path_str));
        ToolContext {
            agent_id: ctx.agent_id.clone(),
            workdir,
            session_id: None,
            call_id: None,
            session: None,
            session_mode,
            manual_background_signal: None,
            media_store: None,
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
        let tool_ctx = Self::tool_context(ctx, self.session_mode);

        // Runtime agent-level tool filtering via query, or no filtering.
        let (agent_tools, agent_disallowed_tools) = if let Some(ref query) = self.tools_config_query
        {
            match query.get_agent_tools_config(&ctx.agent_id).await {
                Some(config) => (config.tools, config.disallowed_tools),
                None => (None, None),
            }
        } else {
            (None, None)
        };

        let params = ToolsSectionParams {
            agent_tools,
            agent_disallowed_tools,
            session_mode: self.session_mode,
            agent_role: self.agent_role.clone(),
            agent_type: self.agent_type.clone(),
        };
        let content = build_tools_section(&self.registry, &tool_ctx, &params).await;

        if content.is_empty() {
            return None;
        }

        Some(PromptFragment {
            section_title: "## Tools".to_string(),
            section_type: SectionType::Tools,
            content,
        })
    }

    /// Cache key for the Tools section.
    ///
    /// Returns a stable key so that `PromptBuilder::build()` can cache
    /// the generated tools listing across repeated builds. Includes the
    /// agent id to avoid cross-agent cache pollution.
    fn cache_key(&self, ctx: &FragmentContext) -> Option<String> {
        Some(format!("tools:{}", ctx.agent_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_adapters::{ApprovalFlowAdapter, PermissionEngineAdapter};
    use closeclaw_permission::engine::engine_types::RuleSet;

    #[test]
    fn test_provider_name_and_priority() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        assert_eq!(provider.name(), "tools");
        assert_eq!(provider.priority(), 2);
    }

    #[test]
    fn test_cache_key_includes_agent_id() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        let mut ctx = FragmentContext::test_default();
        ctx.agent_id = "agent-abc".to_string();
        assert_eq!(
            provider.cache_key(&ctx),
            Some("tools:agent-abc".to_string())
        );
    }

    #[test]
    fn test_cache_key_varies_with_agent_id() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);

        let mut ctx_a = FragmentContext::test_default();
        ctx_a.agent_id = "agent-a".to_string();
        let mut ctx_b = FragmentContext::test_default();
        ctx_b.agent_id = "agent-b".to_string();

        assert_ne!(provider.cache_key(&ctx_a), provider.cache_key(&ctx_b));
    }

    #[tokio::test]
    async fn test_generate_empty_registry_returns_none() {
        let registry = Arc::new(ToolRegistry::new());
        let provider = ToolsFragmentProvider::new(registry, None, None);
        let ctx = FragmentContext::test_default();
        // Empty registry → no tools → content is empty → None
        assert!(provider.generate(&ctx).await.is_none());
    }

    #[tokio::test]
    async fn test_generate_with_tools() {
        let registry = Arc::new(ToolRegistry::new());
        let disk_registry = Arc::new(closeclaw_skills::DiskSkillRegistry::new(vec![]));

        // Register tools via the new Registrar pattern.
        let permission_engine = Arc::new(tokio::sync::RwLock::new(
            closeclaw_permission::engine::engine_eval::PermissionEngine::new_with_default_data_root(
                closeclaw_permission::rules::RuleSetBuilder::new()
                    .build()
                    .unwrap(),
            ),
        ));
        let tmp = tempfile::tempdir().unwrap();
        let cfg_mgr =
            Arc::new(closeclaw_config::ConfigManager::new(tmp.path().to_path_buf()).unwrap());
        let cfg = closeclaw_gateway::GatewayConfig {
            name: "test".to_string(),
            rate_limit_per_minute: 100,
            max_message_size: 65536,
            ..Default::default()
        };
        let session_manager = Arc::new(closeclaw_gateway::SessionManager::new(
            &cfg,
            None,
            None,
            closeclaw_session::persistence::ReasoningLevel::default(),
        ));
        let agent_registry = Arc::new(closeclaw_agent::registry::AgentRegistry::new());
        let spawn_controller = Arc::new(closeclaw_gateway::SpawnController::new(
            Arc::clone(&agent_registry),
            Arc::clone(&cfg_mgr),
            Arc::clone(&session_manager),
            permission_engine.clone(),
        ));

        let task_manager = Arc::new(closeclaw_tasks::BackgroundTaskManager::new());
        let approval_flow = Arc::new(tokio::sync::Mutex::new(
            closeclaw_permission::approval_flow::ApprovalFlow::new(
                Arc::clone(&session_manager) as Arc<dyn closeclaw_common::SessionLookup>,
                Arc::new(|_| {}),
                Arc::new(|_: &str| {}),
                tokio::runtime::Handle::current(),
                closeclaw_permission::approval_flow::HeartbeatApprovalMode::default(),
                tmp.path().to_path_buf(),
                RuleSet::default(),
            ),
        ));
        let registrars: Vec<Box<dyn crate::ToolRegistrar>> = vec![
            Box::new(crate::CoreToolsRegistrar::new(
                permission_engine.clone(),
                task_manager as Arc<dyn closeclaw_tasks::TaskManager>,
                session_manager.clone(),
                cfg_mgr.clone(),
                approval_flow.clone(),
            )),
            Box::new(closeclaw_session::tools::SessionToolsRegistrar::new(
                spawn_controller.clone() as Arc<dyn crate::SpawnValidator>,
                session_manager.clone() as Arc<dyn closeclaw_session::tools::SessionManagerOps>,
                agent_registry.clone() as Arc<dyn closeclaw_agent::AgentConfigLookup>,
                Arc::new(PermissionEngineAdapter(permission_engine)),
                Arc::new(tokio::sync::Mutex::new(ApprovalFlowAdapter(
                    approval_flow.clone(),
                ))),
            )),
            Box::new(crate::SkillsToolsRegistrar::new(Arc::new(
                crate::builtin::SkillTool::new(
                    disk_registry,
                    Arc::new(closeclaw_skills::BuiltinSkillRegistry::new()),
                ),
            ))),
            Box::new(crate::PlanToolsRegistrar::new(
                Arc::new(std::sync::Mutex::new(closeclaw_common::PlanState::new())),
                session_manager.clone(),
                approval_flow.clone(),
            )),
        ];
        registry.register_all(registrars).await.unwrap();

        let provider = ToolsFragmentProvider::new(registry, None, None);
        let ctx = FragmentContext::test_default();
        let fragment = provider.generate(&ctx).await;
        assert!(fragment.is_some());
        let frag = fragment.unwrap();
        assert_eq!(frag.section_type, SectionType::Tools);
        assert!(frag.content.contains("file_ops"));
    }
}
