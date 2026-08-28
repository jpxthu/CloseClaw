//! SystemPromptBuilder production implementation.
//!
//! Bridges the [`SystemPromptBuilder`] trait (from `closeclaw-common`)
//! to the Provider-driven [`PromptBuilder`] pipeline.
//!
//! Implements Step 1.1 of the SystemPromptBuilder production plan.

use async_trait::async_trait;
use closeclaw_agent::lookup::AgentLookup;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::system_prompt::PromptOverrides;
use closeclaw_common::{BootstrapMode, SystemPromptBuilder};
use closeclaw_tools::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::builder::WorkspaceBuildConfig;
use crate::sections::SectionCache;

/// Production implementation of [`SystemPromptBuilder`].
///
/// Wraps the existing [`PromptBuilder`] pipeline to implement the
/// cross-crate trait used by session handlers.
///
/// Holds a shared [`SectionCache`] so that invalidation from any call
/// site (slash handler, compaction callback) reaches
/// all session builders.
pub struct SystemPromptBuilderAdapter {
    tool_registry: Arc<ToolRegistry>,
    agent_registry: Arc<RwLock<AgentRegistry>>,
    workspace_dir: PathBuf,
    shared_cache: Arc<std::sync::RwLock<SectionCache>>,
    skill_listing_provider: Option<Arc<dyn closeclaw_common::SkillListingProvider>>,
}

impl SystemPromptBuilderAdapter {
    /// Create a new adapter instance.
    ///
    /// # Arguments
    /// * `tool_registry` — shared tool registry for ToolsSection generation
    /// * `agent_registry` — shared agent config registry for bootstrap_mode lookup
    /// * `workspace_dir` — root workspace directory; per-agent paths are
    ///   derived as `{workspace_dir}/agents/{agent_id}`
    #[cfg(test)]
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        agent_registry: Arc<RwLock<AgentRegistry>>,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            tool_registry,
            agent_registry,
            workspace_dir,
            shared_cache: Arc::new(std::sync::RwLock::new(SectionCache::new())),
            skill_listing_provider: None,
        }
    }

    /// Create a new adapter with a shared cache instance.
    ///
    /// Used when the caller needs to share the cache across multiple
    /// components (e.g. daemon).
    pub fn new_with_cache(
        tool_registry: Arc<ToolRegistry>,
        agent_registry: Arc<RwLock<AgentRegistry>>,
        workspace_dir: PathBuf,
        shared_cache: Arc<std::sync::RwLock<SectionCache>>,
        skill_listing_provider: Option<Arc<dyn closeclaw_common::SkillListingProvider>>,
    ) -> Self {
        Self {
            tool_registry,
            agent_registry,
            workspace_dir,
            shared_cache,
            skill_listing_provider,
        }
    }

    /// Get a reference to the shared section cache.
    ///
    /// Allows external callers (e.g. daemon) to hold a
    /// clone of the `Arc` and invalidate the same cache.
    pub fn shared_cache(&self) -> &Arc<std::sync::RwLock<SectionCache>> {
        &self.shared_cache
    }

    /// Resolve the agent-level tools config from the registry.
    ///
    /// Returns `(agent_tools, agent_disallowed_tools)` suitable for
    /// [`WorkspaceBuildConfig`]. Filters out the catch-all `"*"` sentinel
    /// and empty lists, normalising them to `None`.
    async fn resolve_agent_tools(
        &self,
        agent_id: &str,
    ) -> (Option<Vec<String>>, Option<Vec<String>>) {
        let guard = self.agent_registry.read().await;
        guard
            .get(agent_id)
            .map(|cfg| {
                let tools = if cfg.tools.is_empty() || cfg.tools == ["*"] {
                    None
                } else {
                    Some(cfg.tools.clone())
                };
                let disallowed = if cfg.disallowed_tools.is_empty() {
                    None
                } else {
                    Some(cfg.disallowed_tools.clone())
                };
                (tools, disallowed)
            })
            .unwrap_or((None, None))
    }
}

#[async_trait]
impl SystemPromptBuilder for SystemPromptBuilderAdapter {
    /// Build a complete system prompt for the given session.
    ///
    /// Resolution order:
    /// 1. If `bootstrap_mode_override` is `Some`, use it.
    /// 2. Otherwise, query `agent_registry` for the agent's configured
    ///    bootstrap mode, falling back to `BootstrapMode::Full`.
    /// 3. Construct the workspace path as `{workspace_dir}/agents/{agent_id}`.
    /// 4. Build the static layer via the Provider-driven pipeline.
    /// 5. Apply [`PromptOverrides`] (override > agent > custom priority).
    async fn build_prompt(
        &self,
        _session_id: &str,
        agent_id: &str,
        overrides: Option<&PromptOverrides>,
        bootstrap_mode_override: Option<BootstrapMode>,
    ) -> String {
        // Step 1: Resolve bootstrap_mode.
        let bootstrap_mode = match bootstrap_mode_override {
            Some(mode) => mode,
            None => {
                let guard = self.agent_registry.read().await;
                guard
                    .query_bootstrap_mode(agent_id)
                    .await
                    .unwrap_or(BootstrapMode::Full)
            }
        };

        // Step 2: Construct workspace path.
        let workspace_path = self.workspace_dir.join("agents").join(agent_id);

        // Step 3: Fetch agent-level tool config for PromptBuilder.
        let (agent_tools, agent_disallowed_tools) = self.resolve_agent_tools(agent_id).await;

        // Step 4: Build static layer via Provider pipeline.
        let config = WorkspaceBuildConfig {
            tool_registry: Some(Arc::clone(&self.tool_registry)),
            agent_id: Some(agent_id.to_string()),
            agent_tools,
            agent_disallowed_tools,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: Some(bootstrap_mode),
            session_mode: None,
            skill_listing_provider: self.skill_listing_provider.clone(),
        };

        let static_layer = crate::builder::build_from_workspace_with_cache(
            &workspace_path,
            config,
            Some(Arc::clone(&self.shared_cache)),
        )
        .await;

        // Step 5: Apply PromptOverrides (override > agent > custom).
        apply_overrides(&static_layer, overrides)
    }

    /// Invalidate all cached prompt sections.
    ///
    /// Called when workspace files, tools, or skills change so the next
    /// `build_prompt()` call regenerates the static layer.
    async fn invalidate_cache(&self) {
        self.shared_cache.write().unwrap().invalidate_all();
    }
}

/// Apply prompt overrides to the built static layer.
///
/// Priority resolution (matching design doc):
/// - `override_prompt` — replaces the entire static layer
/// - `agent_prompt`    — appended after the static layer
/// - `custom_prompt`   — appended after agent_prompt
fn apply_overrides(static_layer: &str, overrides: Option<&PromptOverrides>) -> String {
    let ov = match overrides {
        Some(ov) => ov,
        None => return static_layer.to_string(),
    };

    // override_prompt replaces everything.
    if let Some(ref override_prompt) = ov.override_prompt {
        return override_prompt.clone();
    }

    // agent_prompt or custom_prompt appends to static layer.
    let mut result = static_layer.to_string();
    if let Some(ref agent_prompt) = ov.agent_prompt {
        if !agent_prompt.is_empty() {
            result.push_str("\n\n");
            result.push_str(agent_prompt);
        }
    }
    if let Some(ref custom_prompt) = ov.custom_prompt {
        if !custom_prompt.is_empty() {
            result.push_str("\n\n");
            result.push_str(custom_prompt);
        }
    }
    result
}

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod adapter_tests;
