//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use crate::fragment::{FragmentContext, PromptFragmentProvider};
use crate::sections::{Section, SectionCache};
use closeclaw_common::BootstrapMode;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Re-export the common PromptOverrides type.
pub use closeclaw_common::system_prompt::PromptOverrides;

/// Default system prompt fallback
const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

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
    /// Create a new builder with the given providers.
    ///
    /// Providers are sorted by priority (lower first). The caller is
    /// responsible for constructing the appropriate providers for the
    /// domain crates (tools, skills, memory) and the bootstrap provider
    /// from this crate.
    pub fn new(providers: Vec<Box<dyn PromptFragmentProvider>>) -> Self {
        Self::new_with_cache(providers, Arc::new(RwLock::new(SectionCache::new())))
    }

    /// Create a builder with a shared cache instance.
    ///
    /// Used when the cache must be shared across multiple builders
    /// (e.g. for cross-session invalidation via `SystemPromptBuilder`).
    pub fn new_with_cache(
        mut providers: Vec<Box<dyn PromptFragmentProvider>>,
        shared_cache: Arc<RwLock<SectionCache>>,
    ) -> Self {
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
    /// Pre-constructed provider list (already sorted or will be sorted).
    pub providers: Vec<Box<dyn PromptFragmentProvider>>,

    /// Additional dynamic sections to include.
    pub dynamic_sections: Vec<Section>,
    /// Content to append at the end of the prompt.
    pub append_section: Option<String>,
    /// Bootstrap mode for this build — caller is responsible for querying
    /// the AgentRegistry and passing the result here.
    pub bootstrap_mode_override: Option<BootstrapMode>,
    /// Agent ID — passed through to [`FragmentContext`] so providers can
    /// perform per-agent filtering (tool white/blacklists, skill filtering).
    pub agent_id: Option<String>,
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
        bootstrap_dir: root.to_string_lossy().to_string(),
    };

    let builder = match shared_cache {
        Some(cache) => PromptBuilder::new_with_cache(config.providers, cache),
        None => PromptBuilder::new(config.providers),
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
    fn test_workspace_build_config_has_providers_field() {
        let config = WorkspaceBuildConfig {
            providers: vec![],
            dynamic_sections: vec![],
            append_section: None,
            bootstrap_mode_override: None,
            agent_id: None,
        };
        assert!(config.providers.is_empty());
    }
}
