//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use crate::fragment::{FragmentContext, PromptFragmentProvider};
use crate::providers::bootstrap::BootstrapFragmentProvider;
use crate::providers::memory::MemoryFragmentProvider;
use crate::providers::skills::SkillsFragmentProvider;
use crate::providers::tools::ToolsFragmentProvider;
use crate::sections::{get_cached_section, put_cached_section, Section};
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_common::BootstrapMode;
use closeclaw_skills::DiskSkillRegistry;
use std::path::{Path, PathBuf};
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
/// Holds `Arc` references to the four registries needed by the standard
/// providers and assembles the prompt by asking each provider for its
/// fragment, sorted by priority.
pub struct PromptBuilder {
    agent_registry: Arc<AgentRegistry>,
    tool_registry: Arc<ToolRegistry>,
    skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
    agent_tools: Option<Vec<String>>,
    agent_disallowed_tools: Option<Vec<String>>,
    agent_skills: Option<Vec<String>>,
}

impl PromptBuilder {
    /// Create a new builder with all four registries.
    pub fn new(
        agent_registry: Arc<AgentRegistry>,
        tool_registry: Arc<ToolRegistry>,
        skill_registry: Arc<RwLock<Option<DiskSkillRegistry>>>,
        agent_tools: Option<Vec<String>>,
        agent_disallowed_tools: Option<Vec<String>>,
        agent_skills: Option<Vec<String>>,
    ) -> Self {
        Self {
            agent_registry,
            tool_registry,
            skill_registry,
            agent_tools,
            agent_disallowed_tools,
            agent_skills,
        }
    }

    /// Build the system prompt from the given context.
    ///
    /// Sorts providers by priority, checks section-level cache before
    /// calling `generate()`, skips `None` results, concatenates fragments,
    /// and falls back to `DEFAULT_PROMPT` when no provider contributes.
    pub async fn build(&self, ctx: &FragmentContext) -> String {
        // Create the four standard providers.
        let providers: Vec<Box<dyn PromptFragmentProvider>> = vec![
            Box::new(BootstrapFragmentProvider::new(Arc::clone(
                &self.agent_registry,
            ))),
            Box::new(ToolsFragmentProvider::new(
                Arc::clone(&self.tool_registry),
                self.agent_tools.clone(),
                self.agent_disallowed_tools.clone(),
            )),
            Box::new(SkillsFragmentProvider::new(
                Arc::clone(&self.skill_registry),
                self.agent_skills.clone(),
            )),
            Box::new(MemoryFragmentProvider::new()),
        ];

        // Sort by priority (lower first).
        let mut sorted = providers;
        sorted.sort_by_key(|p| p.priority());

        let mut fragments: Vec<String> = Vec::new();

        for provider in &sorted {
            // Check section-level cache.
            if let Some(key) = provider.cache_key(ctx) {
                if let Some(cached) = get_cached_section(&key, None) {
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
                    put_cached_section(&key, rendered.clone(), None);
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
fn render_section(section: Section) -> String {
    let name = section.name();
    let is_static = section.is_cacheable();

    if is_static {
        match section {
            Section::MemorySection(_) => {
                let path = Path::new("MEMORY.md");
                if path.exists() {
                    crate::sections::load_cached_file_section("memory", path)
                        .map(|c| Section::MemorySection(c).render())
                        .unwrap_or_default()
                } else {
                    section.render()
                }
            }
            Section::HeartbeatSection(_) => {
                let path = Path::new("HEARTBEAT.md");
                if path.exists() {
                    crate::sections::load_cached_file_section("heartbeat", path)
                        .map(|c| Section::HeartbeatSection(c).render())
                        .unwrap_or_default()
                } else {
                    section.render()
                }
            }
            _ => {
                if let Some(cached) = get_cached_section(name, None) {
                    cached
                } else {
                    let rendered = section.render();
                    put_cached_section(name, rendered.clone(), None);
                    rendered
                }
            }
        }
    } else {
        section.render()
    }
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
    /// Skill registry for generating SkillListingSection.
    pub skill_registry: Option<Arc<RwLock<Option<DiskSkillRegistry>>>>,
    /// Agent ID for skill listing filtering.
    pub agent_id: Option<String>,
    /// Agent-level tool whitelist from config (`tools` field).
    pub agent_tools: Option<Vec<String>>,
    /// Agent-level tool blacklist from config (`disallowedTools` field).
    pub agent_disallowed_tools: Option<Vec<String>>,
    /// Agent-level skill whitelist from config (`skills` field).
    pub agent_skills: Option<Vec<String>>,
    /// Additional dynamic sections to include.
    pub dynamic_sections: Vec<Section>,
    /// Content to append at the end of the prompt.
    pub append_section: Option<String>,
    /// Optional AgentRegistry reference for bootstrap mode queries.
    pub agent_registry: Option<Arc<AgentRegistry>>,
    /// Agent directory — when `Some`, used as the bootstrap file loading path
    /// (takes priority over `workspace_root`).
    pub agent_dir: Option<PathBuf>,
    /// Bootstrap mode override — when `Some`, overrides the mode queried
    /// from `AgentRegistry`.
    pub bootstrap_mode_override: Option<BootstrapMode>,
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
    let root = workspace_root.as_ref();

    // Resolve bootstrap mode for FragmentContext.
    // Priority: override value > AgentRegistry query.
    let bootstrap_mode = config.bootstrap_mode_override.or_else(|| {
        config.agent_registry.as_ref().and_then(|reg| {
            config
                .agent_id
                .as_deref()
                .and_then(|id| reg.query_bootstrap_mode(id))
        })
    });

    let ctx = FragmentContext {
        agent_id: config.agent_id.clone(),
        bootstrap_mode,
        workdir: Some(root.to_path_buf()),
        agent_dir: config.agent_dir.clone(),
    };

    let agent_registry = config
        .agent_registry
        .unwrap_or_else(|| Arc::new(AgentRegistry::new()));
    let tool_registry = config
        .tool_registry
        .unwrap_or_else(|| Arc::new(ToolRegistry::new()));
    let skill_registry = config
        .skill_registry
        .unwrap_or_else(|| Arc::new(RwLock::new(None)));

    let builder = PromptBuilder::new(
        agent_registry,
        tool_registry,
        skill_registry,
        config.agent_tools,
        config.agent_disallowed_tools,
        config.agent_skills,
    );

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

    /// Clear cached sections to prevent cross-test pollution.
    #[cfg(test)]
    use crate::sections::invalidate_all_sections;

    fn reset_sections() {
        invalidate_all_sections();
    }

    #[test]
    fn test_prompt_overrides_default() {
        let overrides = PromptOverrides::default();
        assert!(overrides.override_prompt.is_none());
        assert!(overrides.agent_prompt.is_none());
        assert!(overrides.custom_prompt.is_none());
    }

    #[test]
    fn test_build_system_prompt_renders_sections() {
        reset_sections();
        let sections = vec![Section::RoleSection("role content".to_string())];
        let result = build_system_prompt(sections, None);
        assert!(result.contains("role content"));
    }

    #[test]
    fn test_build_system_prompt_fallback_default() {
        reset_sections();
        let sections = vec![];
        let result = build_system_prompt(sections, None);
        assert!(result.contains(DEFAULT_PROMPT));
    }

    #[test]
    fn test_build_system_prompt_with_append() {
        reset_sections();
        let sections = vec![Section::RoleSection("role content".to_string())];
        let result = build_system_prompt(sections, Some("additional info".to_string()));
        assert!(result.contains("role content"));
        assert!(result.contains("additional info"));
        assert!(result.contains("## Append"));
    }

    #[test]
    fn test_build_append_section_appended() {
        reset_sections();
        let sections = vec![Section::RoleSection("base".to_string())];
        let result = build_system_prompt(sections, Some("extra notes".to_string()));
        assert!(result.contains("base"));
        assert!(result.contains("extra notes"));
    }

    #[test]
    fn test_append_section_not_shown_when_empty() {
        reset_sections();
        let sections = vec![Section::RoleSection("base".to_string())];
        let result = build_system_prompt(sections, None);
        assert!(!result.contains("## Append"));
    }

    #[test]
    fn test_dynamic_sections_not_cached() {
        reset_sections();
        let sections = vec![Section::SessionState {
            pending_tasks: vec![],
        }];
        let result1 = build_system_prompt(sections.clone(), None);
        let result2 = build_system_prompt(sections, None);
        assert_eq!(result1, result2);
    }

    // ---- AgentRegistry bootstrap mode query path tests ----

    #[test]
    fn test_workspace_build_config_has_agent_registry_field() {
        let config = WorkspaceBuildConfig {
            tool_registry: None,
            skill_registry: None,
            agent_id: None,
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: None,
            agent_dir: None,
            bootstrap_mode_override: None,
        };
        assert!(config.agent_registry.is_none());
    }

    #[test]
    fn test_workspace_build_config_with_agent_registry() {
        use closeclaw_agent::registry::AgentRegistry;
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
        use closeclaw_session::bootstrap::loader::BootstrapMode;

        let agent_reg = Arc::new(AgentRegistry::new());
        agent_reg.populate(vec![ResolvedAgentConfig {
            id: "test-agent".into(),
            name: "test-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Minimal,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            skill_registry: None,
            agent_id: Some("test-agent".to_string()),
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: Some(agent_reg),
            agent_dir: None,
            bootstrap_mode_override: None,
        };
        assert!(config.agent_registry.is_some());
        assert_eq!(config.agent_id.as_deref(), Some("test-agent"));
    }

    #[test]
    fn test_agent_registry_query_bootstrap_mode_minimal() {
        use closeclaw_agent::registry::AgentRegistry;
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
        use closeclaw_session::bootstrap::loader::BootstrapMode;

        let agent_reg = Arc::new(AgentRegistry::new());
        agent_reg.populate(vec![ResolvedAgentConfig {
            id: "minimal-agent".into(),
            name: "minimal-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Minimal,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let mode = agent_reg.query_bootstrap_mode("minimal-agent");
        assert_eq!(mode, Some(BootstrapMode::Minimal));
    }

    #[test]
    fn test_agent_registry_query_bootstrap_mode_full() {
        use closeclaw_agent::registry::AgentRegistry;
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};
        use closeclaw_session::bootstrap::loader::BootstrapMode;

        let agent_reg = Arc::new(AgentRegistry::new());
        agent_reg.populate(vec![ResolvedAgentConfig {
            id: "full-agent".into(),
            name: "full-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let mode = agent_reg.query_bootstrap_mode("full-agent");
        assert_eq!(mode, Some(BootstrapMode::Full));
    }

    #[test]
    fn test_agent_registry_query_bootstrap_mode_not_found() {
        use closeclaw_agent::registry::AgentRegistry;

        let agent_reg = Arc::new(AgentRegistry::new());
        let mode = agent_reg.query_bootstrap_mode("missing-agent");
        assert_eq!(mode, None);
    }

    // ---- PromptBuilder tests ----

    #[test]
    fn test_prompt_builder_new() {
        let agent_reg = Arc::new(AgentRegistry::new());
        let tool_reg = Arc::new(ToolRegistry::new());
        let skill_reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let builder = PromptBuilder::new(agent_reg, tool_reg, skill_reg, None, None, None);
        // Just verify construction succeeds.
        assert!(builder.agent_tools.is_none());
    }

    #[tokio::test]
    async fn test_prompt_builder_build_fallback_default() {
        let agent_reg = Arc::new(AgentRegistry::new());
        let tool_reg = Arc::new(ToolRegistry::new());
        let skill_reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let builder = PromptBuilder::new(agent_reg, tool_reg, skill_reg, None, None, None);

        // No workdir → BootstrapFragmentProvider returns None
        // Empty tool registry → ToolsFragmentProvider returns None
        // Empty skill registry → SkillsFragmentProvider returns None
        // No workdir → MemoryFragmentProvider returns None
        // → fallback DEFAULT_PROMPT
        let ctx = FragmentContext::default();
        let result = builder.build(&ctx).await;
        assert_eq!(result, DEFAULT_PROMPT);
    }

    #[tokio::test]
    async fn test_prompt_builder_build_with_memory() {
        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MEMORY.md"), "remember X").unwrap();

        let agent_reg = Arc::new(AgentRegistry::new());
        let tool_reg = Arc::new(ToolRegistry::new());
        let skill_reg = Arc::new(RwLock::new(Some(DiskSkillRegistry::new(vec![]))));
        let builder = PromptBuilder::new(agent_reg, tool_reg, skill_reg, None, None, None);

        let ctx = FragmentContext {
            workdir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let result = builder.build(&ctx).await;
        assert!(result.contains("## Memory"));
        assert!(result.contains("remember X"));
    }

    // ---- bootstrap_mode_override tests ----

    #[tokio::test]
    async fn test_build_from_workspace_override_mode_overrides_registry() {
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};

        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        // BOOTSTRAP.md is only loaded in Full mode, not Minimal.
        std::fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();

        let agent_reg = Arc::new(AgentRegistry::new());
        agent_reg.populate(vec![ResolvedAgentConfig {
            id: "test-agent".into(),
            name: "test-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            skill_registry: None,
            agent_id: Some("test-agent".into()),
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: Some(agent_reg),
            agent_dir: None,
            bootstrap_mode_override: Some(BootstrapMode::Minimal),
        };

        let result = build_from_workspace(tmp.path(), config).await;
        // Override forces Minimal → BOOTSTRAP.md excluded.
        assert!(!result.contains("bootstrap only in full"));
        assert!(result.contains("agents content"));
    }

    #[tokio::test]
    async fn test_build_from_workspace_no_override_uses_registry() {
        use closeclaw_config::agents::{ConfigSource, ResolvedAgentConfig};

        crate::sections::invalidate_all_sections();
        let tmp = tempfile::tempdir().unwrap();
        // BOOTSTRAP.md is only loaded in Full mode.
        std::fs::write(tmp.path().join("BOOTSTRAP.md"), "bootstrap only in full").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "agents content").unwrap();

        let agent_reg = Arc::new(AgentRegistry::new());
        agent_reg.populate(vec![ResolvedAgentConfig {
            id: "test-agent".into(),
            name: "test-agent".into(),
            parent_id: None,
            model: None,
            workspace: None,
            agent_dir: None,
            bootstrap_mode: BootstrapMode::Full,
            skills: vec![],
            tools: vec![],
            disallowed_tools: vec![],
            subagents: Default::default(),
            memory: None,
            source: ConfigSource::User,
        }]);

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            skill_registry: None,
            agent_id: Some("test-agent".into()),
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: Some(agent_reg),
            agent_dir: None,
            bootstrap_mode_override: None,
        };

        let result = build_from_workspace(tmp.path(), config).await;
        // No override → registry Full → BOOTSTRAP.md included.
        assert!(result.contains("bootstrap only in full"));
    }

    #[tokio::test]
    async fn test_build_from_workspace_agent_dir_preferred() {
        crate::sections::invalidate_all_sections();
        let workdir_tmp = tempfile::tempdir().unwrap();
        let agent_tmp = tempfile::tempdir().unwrap();
        std::fs::write(workdir_tmp.path().join("AGENTS.md"), "from workdir").unwrap();
        std::fs::write(agent_tmp.path().join("AGENTS.md"), "from agent_dir").unwrap();

        let config = WorkspaceBuildConfig {
            tool_registry: None,
            skill_registry: None,
            agent_id: None,
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: None,
            agent_dir: Some(agent_tmp.path().to_path_buf()),
            bootstrap_mode_override: Some(BootstrapMode::Minimal),
        };

        let result = build_from_workspace(workdir_tmp.path(), config).await;
        assert!(result.contains("from agent_dir"));
        assert!(!result.contains("from workdir"));
    }
}
