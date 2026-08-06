//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use crate::fragment::{FragmentContext, PromptFragmentProvider};
use crate::providers::bootstrap::BootstrapFragmentProvider;
use crate::providers::memory::MemoryFragmentProvider;
use crate::providers::skills::SkillsFragmentProvider;
use crate::providers::tools::ToolsFragmentProvider;
use crate::sections::{Section, SectionCache};
use closeclaw_common::session_mode::SessionMode;
use closeclaw_common::BootstrapMode;
use closeclaw_common::SkillListingProvider;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Re-export the common PromptOverrides type.
pub use closeclaw_common::system_prompt::PromptOverrides;

/// Default system prompt fallback
const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

use closeclaw_tools::ToolRegistry;

// ---------------------------------------------------------------------------
// PromptBuilder: Provider-driven prompt assembly
// ---------------------------------------------------------------------------

/// Provider-driven system prompt builder.
///
/// Holds a list of registered [`PromptFragmentProvider`] instances,
/// sorted by priority. Providers are created once at construction time
/// and reused across all `build()` invocations.
pub struct PromptBuilder {
    providers: Vec<Box<dyn PromptFragmentProvider>>,
    cache: Arc<RwLock<SectionCache>>,
}

impl PromptBuilder {
    /// Create a new builder with the standard providers registered.
    ///
    /// The three standard providers (Bootstrap, Tools, Memory) are created
    /// here and sorted by priority. They are reused across all `build()`
    /// invocations.
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        agent_tools: Option<Vec<String>>,
        agent_disallowed_tools: Option<Vec<String>>,
        session_mode: Option<SessionMode>,
        skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,
    ) -> Self {
        Self::new_with_cache(
            tool_registry,
            agent_tools,
            agent_disallowed_tools,
            session_mode,
            Arc::new(RwLock::new(SectionCache::new())),
            skill_listing_provider,
        )
    }

    /// Create a builder with a shared cache instance.
    ///
    /// Used when the cache must be shared across multiple builders
    /// (e.g. for cross-session invalidation via `SystemPromptBuilder`).
    pub fn new_with_cache(
        tool_registry: Arc<ToolRegistry>,
        agent_tools: Option<Vec<String>>,
        agent_disallowed_tools: Option<Vec<String>>,
        session_mode: Option<SessionMode>,
        shared_cache: Arc<RwLock<SectionCache>>,
        skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,
    ) -> Self {
        let mut providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
            Box::new(BootstrapFragmentProvider::new()),
            Box::new(ToolsFragmentProvider::new(
                Arc::clone(&tool_registry),
                agent_tools,
                agent_disallowed_tools,
                session_mode,
            )),
        ];
        if let Some(listing) = skill_listing_provider {
            providers.push(Box::new(SkillsFragmentProvider::new(listing)));
        }
        providers.push(Box::new(MemoryFragmentProvider::new()));
        providers.sort_by_key(|p| p.priority());

        Self {
            providers,
            cache: shared_cache,
        }
    }

    /// Get a reference to the shared cache for external invalidation.
    pub fn shared_cache(&self) -> &Arc<RwLock<SectionCache>> {
        &self.cache
    }

    /// Build the system prompt from the given context.
    ///
    /// Iterates the pre-registered providers, checks section-level cache
    /// before calling `generate()`, skips `None` results, concatenates
    /// fragments, and falls back to `DEFAULT_PROMPT` when no provider
    /// contributes.
    pub async fn build(&self, ctx: &FragmentContext) -> String {
        let mut fragments: Vec<String> = Vec::new();

        for provider in &self.providers {
            // Check section-level cache.
            if let Some(key) = provider.cache_key(ctx) {
                let cache = self.cache.read().unwrap();
                if let Some(cached) = cache.get(&key, None) {
                    fragments.push(cached);
                    continue;
                }
            }

            if let Some(fragment) = provider.generate(ctx).await {
                let rendered = if fragment.section_title.is_empty() {
                    format!("{}\n", fragment.content)
                } else {
                    format!("{}\n{}\n", fragment.section_title, fragment.content)
                };
                // Cache the rendered fragment.
                if let Some(key) = provider.cache_key(ctx) {
                    self.cache
                        .write()
                        .unwrap()
                        .put(&key, rendered.clone(), None);
                }
                fragments.push(rendered);
            }
        }

        if fragments.is_empty() {
            DEFAULT_PROMPT.to_string()
        } else {
            fragments.join("\n")
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy / compat entry points
// ---------------------------------------------------------------------------

/// Build the complete system prompt from the given sections.
///
/// This function only renders sections and appends the optional `append_section`.
/// Priority-prompt resolution (override > agent > custom) is handled at the
/// request stage by [`build_full_system_prompt`] in this module's [`inject`].
pub fn build_system_prompt(sections: Vec<Section>, append_section: Option<String>) -> String {
    let rendered = render_sections(sections);
    let base = if rendered.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        rendered.join("\n")
    };

    append_append_section(base, append_section)
}

/// Render all sections into a vector of strings.
fn render_sections(sections: Vec<Section>) -> Vec<String> {
    sections.into_iter().map(render_section).collect()
}

/// Render a single section to string.
///
/// In the provider-driven pipeline, MemorySection is handled by
/// [`MemoryFragmentProvider`]. This function is only called for dynamic
/// sections in `build_from_workspace` and the legacy `build_system_prompt`.
fn render_section(section: Section) -> String {
    section.render()
}

/// Append the current append_section to a base prompt.
fn append_append_section(base: String, append: Option<String>) -> String {
    if let Some(append) = append {
        format!("{}\n\n## Append\n{}\n", base, append)
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Convenience: build from file-based workspace sections
// ---------------------------------------------------------------------------

/// Configuration for `build_from_workspace`.
pub struct WorkspaceBuildConfig {
    /// Tool registry for generating the ToolsSection.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Agent ID for prompt context.
    pub agent_id: Option<String>,
    /// Agent-level tool whitelist from config (`tools` field).
    pub agent_tools: Option<Vec<String>>,
    /// Agent-level tool blacklist from config (`disallowedTools` field).
    pub agent_disallowed_tools: Option<Vec<String>>,
    /// Skill listing provider for the skills section.
    /// When `Some`, a [`SkillsFragmentProvider`] is registered in the
    /// provider pipeline (priority=3).
    pub skill_listing_provider: Option<Arc<dyn SkillListingProvider>>,

    /// Additional dynamic sections to include.
    pub dynamic_sections: Vec<Section>,
    /// Content to append at the end of the prompt.
    pub append_section: Option<String>,
    /// Bootstrap mode for this build — caller is responsible for querying
    /// the AgentRegistry and passing the result here.
    pub bootstrap_mode_override: Option<BootstrapMode>,
    /// Session mode for mode-aware tool filtering.
    pub session_mode: Option<SessionMode>,
    /// Effective spawn depth budget for the current session.
    ///
    /// When `Some(budget)` where `budget ≤ 0`, the `sessions_spawn`
    /// tool is filtered out of the visible tool list.
    pub effective_spawn_budget: Option<u32>,
}

// --- Private helpers -------------------------------------------------------

/// Build a system prompt from a workspace directory.
///
/// Internally constructs a [`FragmentContext`] and [`PromptBuilder`],
/// delegating the actual assembly to the Provider-driven pipeline.
/// The public signature and return value are unchanged.
pub async fn build_from_workspace<P: AsRef<Path>>(
    workspace_root: P,
    config: WorkspaceBuildConfig,
) -> String {
    build_from_workspace_with_cache(workspace_root, config, None).await
}

/// Build a system prompt from a workspace directory with a shared cache.
///
/// When `shared_cache` is `Some`, the builder reuses the provided cache
/// instance (for cross-session invalidation). Otherwise creates a fresh,
/// isolated cache (default behavior).
pub async fn build_from_workspace_with_cache<P: AsRef<Path>>(
    workspace_root: P,
    config: WorkspaceBuildConfig,
    shared_cache: Option<Arc<RwLock<SectionCache>>>,
) -> String {
    let root = workspace_root.as_ref();

    // Resolve bootstrap mode for FragmentContext.
    // The caller is responsible for querying the AgentRegistry and passing
    // the bootstrap mode via `bootstrap_mode_override`.
    let bootstrap_mode = config.bootstrap_mode_override;

    let ctx = FragmentContext {
        agent_id: config.agent_id.clone().unwrap_or_default(),
        bootstrap_mode: bootstrap_mode.unwrap_or(BootstrapMode::Full),
        bootstrap_dir: root.to_path_buf(),
        effective_spawn_budget: config.effective_spawn_budget,
    };

    let tool_registry = config
        .tool_registry
        .unwrap_or_else(|| Arc::new(ToolRegistry::new()));

    let builder = match shared_cache {
        Some(cache) => PromptBuilder::new_with_cache(
            tool_registry,
            config.agent_tools,
            config.agent_disallowed_tools,
            config.session_mode,
            cache,
            config.skill_listing_provider,
        ),
        None => PromptBuilder::new(
            tool_registry,
            config.agent_tools,
            config.agent_disallowed_tools,
            config.session_mode,
            config.skill_listing_provider,
        ),
    };

    let static_layer = builder.build(&ctx).await;

    // Render dynamic sections (not cached, always rebuilt).
    let dynamic_rendered: Vec<String> = config
        .dynamic_sections
        .into_iter()
        .map(render_section)
        .collect();

    let mut all_parts = Vec::new();
    if static_layer != DEFAULT_PROMPT {
        all_parts.push(static_layer);
    }
    all_parts.extend(dynamic_rendered);

    let base = if all_parts.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        all_parts.join("\n")
    };

    append_append_section(base, config.append_section)
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod builder_tests;

#[cfg(test)]
mod tests {
    use super::super::sections::Section;
    use super::*;

    #[test]
    fn test_prompt_overrides_default() {
        let overrides = PromptOverrides::default();
        assert!(overrides.override_prompt.is_none());
        assert!(overrides.agent_prompt.is_none());
        assert!(overrides.custom_prompt.is_none());
    }

    #[test]
    fn test_build_system_prompt_renders_sections() {
        let sections = vec![Section::MemorySection("memory content".to_string())];
        let result = build_system_prompt(sections, None);
        assert!(result.contains("memory content"));
    }

    #[test]
    fn test_build_system_prompt_fallback_default() {
        let sections = vec![];
        let result = build_system_prompt(sections, None);
        assert!(result.contains(DEFAULT_PROMPT));
    }

    #[test]
    fn test_build_system_prompt_with_append() {
        let sections = vec![Section::MemorySection("memory content".to_string())];
        let result = build_system_prompt(sections, Some("additional info".to_string()));
        assert!(result.contains("memory content"));
        assert!(result.contains("additional info"));
        assert!(result.contains("## Append"));
    }

    #[test]
    fn test_build_append_section_appended() {
        let sections = vec![Section::MemorySection("base".to_string())];
        let result = build_system_prompt(sections, Some("extra notes".to_string()));
        assert!(result.contains("base"));
        assert!(result.contains("extra notes"));
    }

    #[test]
    fn test_append_section_not_shown_when_empty() {
        let sections = vec![Section::MemorySection("base".to_string())];
        let result = build_system_prompt(sections, None);
        assert!(!result.contains("## Append"));
    }

    #[test]
    fn test_dynamic_sections_not_cached() {
        let sections = vec![Section::ChannelContext {
            chat_name: "test".into(),
        }];
        let result1 = build_system_prompt(sections.clone(), None);
        let result2 = build_system_prompt(sections, None);
        assert_eq!(result1, result2);
    }

    // ---- WorkspaceBuildConfig tests ----

    #[test]
    fn test_workspace_build_config_has_agent_id_field() {
        let config = WorkspaceBuildConfig {
            tool_registry: None,
            agent_id: None,
            agent_tools: None,
            agent_disallowed_tools: None,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: None,
            session_mode: None,
            effective_spawn_budget: None,
            skill_listing_provider: None,
        };
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_workspace_build_config_with_agent_id() {
        use closeclaw_session::bootstrap::loader::BootstrapMode;

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            agent_id: Some("test-agent".to_string()),
            agent_tools: None,
            agent_disallowed_tools: None,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: Some(BootstrapMode::Minimal),
            session_mode: None,
            effective_spawn_budget: None,
            skill_listing_provider: None,
        };
        assert_eq!(config.agent_id.as_deref(), Some("test-agent"));
        assert_eq!(config.bootstrap_mode_override, Some(BootstrapMode::Minimal));
    }

    // ---- PromptBuilder tests ----

    #[test]
    fn test_prompt_builder_new() {
        let tool_reg = Arc::new(ToolRegistry::new());
        let builder = PromptBuilder::new(tool_reg, None, None, None, None);
        // Verify construction succeeds, providers are registered, and list is non-empty.
        assert!(!builder.providers.is_empty());
        assert_eq!(builder.providers.len(), 3);
    }

    #[test]
    fn test_prompt_builder_providers_sorted_by_priority() {
        let tool_reg = Arc::new(ToolRegistry::new());
        let builder = PromptBuilder::new(tool_reg, None, None, None, None);
        let priorities: Vec<u32> = builder.providers.iter().map(|p| p.priority()).collect();
        // Bootstrap=1, Tools=2, Memory=4 (no Skills provider when None)
        assert_eq!(priorities, vec![1, 2, 4]);
        // Verify provider names match expected order.
        assert_eq!(builder.providers[0].name(), "bootstrap");
        assert_eq!(builder.providers[1].name(), "tools");
        assert_eq!(builder.providers[2].name(), "memory");
    }

    #[tokio::test]
    async fn test_prompt_builder_build_fallback_default() {
        let tool_reg = Arc::new(ToolRegistry::new());
        let builder = PromptBuilder::new(tool_reg, None, None, None, None);

        // No bootstrap_dir → BootstrapFragmentProvider returns None
        // Empty tool registry → ToolsFragmentProvider returns None
        // No bootstrap_dir → MemoryFragmentProvider returns None
        // → fallback DEFAULT_PROMPT
        let ctx = FragmentContext::test_default();
        let result = builder.build(&ctx).await;
        assert_eq!(result, DEFAULT_PROMPT);
    }

    #[tokio::test]
    async fn test_prompt_builder_build_with_memory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "remember X").unwrap();

        let tool_reg = Arc::new(ToolRegistry::new());
        let builder = PromptBuilder::new(tool_reg, None, None, None, None);

        let ctx = FragmentContext {
            bootstrap_dir: tmp.path().to_path_buf(),
            ..FragmentContext::test_default()
        };
        let result = builder.build(&ctx).await;
        assert!(result.contains("## Memory"));
        assert!(result.contains("remember X"));
    }

    // ---- bootstrap_mode_override tests ----

    #[tokio::test]
    async fn test_build_from_workspace_override_mode() {
        let tmp = tempfile::tempdir().unwrap();
        // BOOTSTRAP.md is only loaded in Full mode, not Minimal.
        std::fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            agent_id: Some("test-agent".into()),
            agent_tools: None,
            agent_disallowed_tools: None,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: Some(BootstrapMode::Minimal),
            session_mode: None,
            effective_spawn_budget: None,
            skill_listing_provider: None,
        };

        let result = build_from_workspace(tmp.path(), config).await;
        // Override forces Minimal → BOOTSTRAP.md excluded.
        assert!(!result.contains("bootstrap only in full"));
        assert!(result.contains("agents content"));
    }

    #[tokio::test]
    async fn test_build_from_workspace_no_override_defaults_to_full() {
        let tmp = tempfile::tempdir().unwrap();
        // BOOTSTRAP.md is only loaded in Full mode.
        std::fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            agent_id: Some("test-agent".into()),
            agent_tools: None,
            agent_disallowed_tools: None,
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: None,
            session_mode: None,
            effective_spawn_budget: None,
            skill_listing_provider: None,
        };

        let result = build_from_workspace(tmp.path(), config).await;
        // No override → defaults to Full → BOOTSTRAP.md included.
        assert!(result.contains("bootstrap only in full"));
    }
}
