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
use closeclaw_common::{BootstrapMode, PromptFragmentProvider, SystemPromptBuilder};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::builder::WorkspaceBuildConfig;
use crate::sections::SectionCache;

#[cfg(test)]
use crate::providers::bootstrap::BootstrapFragmentProvider;
#[cfg(test)]
use closeclaw_common::SkillListingProvider;
#[cfg(test)]
use closeclaw_tools::ToolRegistry;

/// Production implementation of [`SystemPromptBuilder`].
///
/// Wraps the existing [`PromptBuilder`] pipeline to implement the
/// cross-crate trait used by session handlers.
///
/// Holds a shared [`SectionCache`] so that invalidation from any call
/// site (slash handler, compaction callback) reaches
/// all session builders.
pub struct SystemPromptBuilderAdapter {
    agent_registry: Arc<RwLock<AgentRegistry>>,
    workspace_dir: PathBuf,
    shared_cache: Arc<std::sync::RwLock<SectionCache>>,
    /// Pre-constructed providers for each build call.
    /// The caller (daemon) is responsible for building this list from
    /// the domain crates (tools, skills, memory) and BootstrapFragmentProvider.
    providers: Vec<Arc<dyn PromptFragmentProvider>>,
}

impl SystemPromptBuilderAdapter {
    /// Create a new adapter with pre-constructed providers.
    ///
    /// # Arguments
    /// * `agent_registry` — shared agent config registry for bootstrap_mode lookup
    /// * `workspace_dir` — root workspace directory; per-agent paths are
    ///   derived as `{workspace_dir}/agents/{agent_id}`
    /// * `providers` — pre-constructed provider list (will be sorted by priority)
    #[cfg(test)]
    pub fn new(
        agent_registry: Arc<RwLock<AgentRegistry>>,
        workspace_dir: PathBuf,
        providers: Vec<Arc<dyn PromptFragmentProvider>>,
    ) -> Self {
        Self {
            agent_registry,
            workspace_dir,
            shared_cache: Arc::new(std::sync::RwLock::new(SectionCache::new())),
            providers,
        }
    }

    /// Create a new adapter with pre-constructed providers and a shared cache.
    ///
    /// Used when the caller needs to share the cache across multiple
    /// components (e.g. daemon).
    pub fn new_with_providers(
        agent_registry: Arc<RwLock<AgentRegistry>>,
        workspace_dir: PathBuf,
        shared_cache: Arc<std::sync::RwLock<SectionCache>>,
        providers: Vec<Arc<dyn PromptFragmentProvider>>,
    ) -> Self {
        Self {
            agent_registry,
            workspace_dir,
            shared_cache,
            providers,
        }
    }

    /// Legacy constructor — kept for test convenience.
    #[cfg(test)]
    pub fn new_with_cache(
        tool_registry: Arc<ToolRegistry>,
        agent_registry: Arc<RwLock<AgentRegistry>>,
        workspace_dir: PathBuf,
        shared_cache: Arc<std::sync::RwLock<SectionCache>>,
        skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,
    ) -> Self {
        let mut providers: Vec<Arc<dyn PromptFragmentProvider>> =
            vec![Arc::new(BootstrapFragmentProvider::new())];
        if let Some(listing) = skill_listing_provider {
            providers.push(Arc::new(closeclaw_skills::SkillsFragmentProvider::new(
                listing,
            )));
        }
        providers.push(Arc::new(closeclaw_memory::MemoryFragmentProvider::new()));
        providers.push(Arc::new(closeclaw_tools::ToolsFragmentProvider::new(
            tool_registry,
            None,
            None,
        )));
        providers.sort_by_key(|p| p.priority());

        Self {
            agent_registry,
            workspace_dir,
            shared_cache,
            providers,
        }
    }

    /// Get a reference to the shared section cache.
    ///
    /// Allows external callers (e.g. daemon) to hold a
    /// clone of the `Arc` and invalidate the same cache.
    pub fn shared_cache(&self) -> &Arc<std::sync::RwLock<SectionCache>> {
        &self.shared_cache
    }
}

/// Wrapper that adapts an `Arc<dyn PromptFragmentProvider>` into a
/// `Box<dyn PromptFragmentProvider>` by delegating all trait methods.
///
/// This allows the adapter to share providers via `Arc` while still
/// providing `Box`-based providers to `PromptBuilder`.
struct ArcProviderAdapter {
    inner: Arc<dyn PromptFragmentProvider>,
}

impl ArcProviderAdapter {
    fn new(inner: Arc<dyn PromptFragmentProvider>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl PromptFragmentProvider for ArcProviderAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn priority(&self) -> u32 {
        self.inner.priority()
    }

    async fn generate(
        &self,
        ctx: &closeclaw_common::FragmentContext,
    ) -> Option<closeclaw_common::PromptFragment> {
        self.inner.generate(ctx).await
    }

    fn cache_key(&self, ctx: &closeclaw_common::FragmentContext) -> Option<String> {
        self.inner.cache_key(ctx)
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

        // Step 3: Build static layer via Provider pipeline.
        // Wrap Arc providers into Box for PromptBuilder.
        let providers: Vec<Box<dyn PromptFragmentProvider>> = self
            .providers
            .iter()
            .map(|p| {
                Box::new(ArcProviderAdapter::new(Arc::clone(p))) as Box<dyn PromptFragmentProvider>
            })
            .collect();

        let config = WorkspaceBuildConfig {
            providers,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: Some(bootstrap_mode),
            agent_id: Some(agent_id.to_string()),
        };

        let static_layer = crate::builder::build_from_workspace_with_cache(
            &workspace_path,
            config,
            Some(Arc::clone(&self.shared_cache)),
        )
        .await;

        // Step 4: Apply PromptOverrides (override > agent > custom).
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
