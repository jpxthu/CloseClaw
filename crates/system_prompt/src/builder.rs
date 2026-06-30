//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use crate::sections::{get_cached_section, load_cached_file_section, read_file_section, Section};
use crate::tools_section::build_tools_section;
use closeclaw_agent::registry::AgentRegistry;
use closeclaw_session::bootstrap::loader::load_bootstrap_files;
use closeclaw_skills::DiskSkillRegistry;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Re-export the common PromptOverrides type.
pub use closeclaw_common::system_prompt::PromptOverrides;

/// Default system prompt fallback
const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Build the complete system prompt from the given sections.
///
/// This function only renders sections and appends the optional `append_section`.
/// Priority-prompt resolution (override > agent > custom) is handled at the
/// request stage by [`build_full_system_prompt`] in this module's [`inject`].
///
///  1. Renders all provided sections
///  2. Falls back to DEFAULT_PROMPT when no sections produce output
///  3. Appends `append_section` if provided
pub fn build_system_prompt(sections: Vec<Section>, append_section: Option<String>) -> String {
    // Render sections
    let rendered = render_sections(sections);
    let base = if rendered.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        rendered.join("\n")
    };

    append_append_section(base, append_section)
}

/// Render all sections into a vector of strings
fn render_sections(sections: Vec<Section>) -> Vec<String> {
    sections.into_iter().map(render_section).collect()
}

/// Render a single section to string
fn render_section(section: Section) -> String {
    let name = section.name();
    let is_static = section.is_cacheable();

    if is_static {
        match section {
            Section::MemorySection(_) => {
                let path = Path::new("MEMORY.md");
                if path.exists() {
                    load_cached_file_section("memory", path)
                        .map(|c| Section::MemorySection(c).render())
                        .unwrap_or_default()
                } else {
                    section.render()
                }
            }
            Section::HeartbeatSection(_) => {
                let path = Path::new("HEARTBEAT.md");
                if path.exists() {
                    load_cached_file_section("heartbeat", path)
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
                    crate::sections::put_cached_section(name, rendered.clone(), None);
                    rendered
                }
            }
        }
    } else {
        section.render()
    }
}

/// Append the current append_section to a base prompt
fn append_append_section(base: String, append: Option<String>) -> String {
    if let Some(append) = append {
        format!("{}\n\n## Append\n{}\n", base, append)
    } else {
        base
    }
}

use closeclaw_tools::{ToolContext, ToolRegistry};

// ---------------------------------------------------------------------------
// Convenience: build from file-based workspace sections
// ---------------------------------------------------------------------------

/// Configuration for `build_from_workspace`.
pub struct WorkspaceBuildConfig<'a> {
    /// Bootstrap files as (filename, content) pairs, in display order.
    /// Provided by `load_bootstrap_files`.
    ///
    /// When empty and `agent_registry` + `agent_id` are set, the builder
    /// queries the AgentRegistry for the bootstrap mode and loads files
    /// automatically (design-doc query path).
    pub bootstrap_files: Vec<(String, String)>,
    /// Tool registry for generating the ToolsSection.
    pub tool_registry: Option<&'a ToolRegistry>,
    /// Tool context for tool section rendering.
    pub tool_ctx: &'a ToolContext,
    /// Skill registry for generating SkillListingSection.
    pub skill_registry: Option<Arc<RwLock<Option<DiskSkillRegistry>>>>,
    /// Agent ID for skill listing filtering.
    pub agent_id: Option<&'a str>,
    /// Agent-level tool whitelist from config (`tools` field).
    ///
    /// Passed through to [`PromptGenerationContext`] so the tool
    /// section only lists tools the agent is allowed to use.
    pub agent_tools: Option<Vec<String>>,
    /// Agent-level tool blacklist from config (`disallowedTools` field).
    pub agent_disallowed_tools: Option<Vec<String>>,
    /// Agent-level skill whitelist from config (`skills` field).
    ///
    /// When set, only skills whose names appear in the list are included
    /// in the system prompt skill listing. A value of `["*"]` means no
    /// filtering (all skills are shown).
    pub agent_skills: Option<Vec<String>>,
    /// Additional dynamic sections to include.
    pub dynamic_sections: Vec<Section>,
    /// Content to append at the end of the prompt.
    pub append_section: Option<String>,
    /// Optional AgentRegistry reference for direct bootstrap mode queries.
    ///
    /// When set alongside `agent_id`, the builder can query the AgentRegistry
    /// for the agent's bootstrap mode configuration, fulfilling the
    /// design-doc query path: System Prompt → AgentRegistry.get(agent_id)
    /// → bootstrap_mode.
    pub agent_registry: Option<Arc<AgentRegistry>>,
}

// --- Private helpers -------------------------------------------------------

/// Build the RoleSection from bootstrap files.
///
/// Filters out `MEMORY.md` entries (handled separately by the
/// MemorySection path), concatenates the remaining bootstrap file
/// contents, and pushes a `RoleSection` if any content exists.
fn build_role_section(sections: &mut Vec<Section>, bootstrap_files: &[(String, String)]) {
    let role: String = bootstrap_files
        .iter()
        .filter(|(n, _)| n != "MEMORY.md")
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !role.is_empty() {
        sections.push(Section::RoleSection(role));
    }
}
fn push_skill_listing_section(
    sections: &mut Vec<Section>,
    skill_registry: &Option<Arc<RwLock<Option<DiskSkillRegistry>>>>,
    agent_id: Option<&str>,
    agent_skills: Option<&[String]>,
) {
    let Some(lock) = skill_registry else {
        return;
    };
    let Ok(g) = lock.read() else {
        return;
    };
    let Some(reg) = g.as_ref() else {
        return;
    };
    let listing = reg.generate_listing(agent_id, agent_skills);
    if !listing.is_empty() {
        sections.push(Section::SkillListingSection(listing));
    }
}

/// Resolve bootstrap files: use pre-loaded files, or query AgentRegistry
/// for the bootstrap mode and load them on the fly.
fn resolve_bootstrap_files(
    root: &Path,
    config: &WorkspaceBuildConfig<'_>,
) -> Vec<(String, String)> {
    if !config.bootstrap_files.is_empty() {
        return config.bootstrap_files.clone();
    }
    let Some(registry) = config.agent_registry.as_ref() else {
        return config.bootstrap_files.clone();
    };
    let Some(agent_id) = config.agent_id else {
        return config.bootstrap_files.clone();
    };
    registry
        .query_bootstrap_mode(agent_id)
        .map(|mode| {
            load_bootstrap_files(root, mode)
                .unwrap_or_default()
                .into_iter()
                .collect()
        })
        .unwrap_or_else(|| config.bootstrap_files.clone())
}

/// Build a system prompt from a workspace directory.
///
/// When `config.bootstrap_files` is empty and `config.agent_registry`
/// and `config.agent_id` are set, the builder queries the AgentRegistry
/// for the agent's bootstrap mode and loads files automatically
/// (design-doc query path: System Prompt → AgentRegistry).
///
/// Push the SkillListingSection into `sections` when a skill registry
/// is available and produces a non-empty listing.
pub async fn build_from_workspace<P: AsRef<Path>>(
    workspace_root: P,
    config: WorkspaceBuildConfig<'_>,
) -> String {
    let root = workspace_root.as_ref();
    let mut sections: Vec<Section> = Vec::new();

    let resolved_bootstrap_files = resolve_bootstrap_files(root, &config);
    build_role_section(&mut sections, &resolved_bootstrap_files);
    // MemorySection — skip if workspace_root is missing/empty
    if root.exists() && root.is_dir() {
        let memory_path = root.join("MEMORY.md");
        if let Some((content, _)) = read_file_section(&memory_path) {
            if !content.is_empty() {
                sections.push(Section::MemorySection(content));
            }
        }
    }
    // ToolsSection — real content when registry available
    if let Some(reg) = config.tool_registry {
        sections.push(
            build_tools_section(
                reg,
                config.tool_ctx,
                config.agent_tools.clone(),
                config.agent_disallowed_tools.clone(),
            )
            .await,
        );
    } else {
        sections.push(Section::ToolsSection(String::new()));
    }
    // SkillListingSection
    push_skill_listing_section(
        &mut sections,
        &config.skill_registry,
        config.agent_id,
        config.agent_skills.as_deref(),
    );
    sections.extend(config.dynamic_sections);
    build_system_prompt(sections, config.append_section)
}

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
        // Verify the agent_registry field is accessible and defaults to None
        let ctx = ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let config = WorkspaceBuildConfig {
            bootstrap_files: vec![],
            tool_registry: None,
            tool_ctx: &ctx,
            skill_registry: None,
            agent_id: None,
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: None,
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

        let ctx = ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let config = WorkspaceBuildConfig {
            bootstrap_files: vec![],
            tool_registry: None,
            tool_ctx: &ctx,
            skill_registry: None,
            agent_id: Some("test-agent"),
            agent_tools: None,
            agent_disallowed_tools: None,
            agent_skills: None,
            dynamic_sections: vec![],
            append_section: None,
            agent_registry: Some(agent_reg),
        };
        assert!(config.agent_registry.is_some());
        assert_eq!(config.agent_id, Some("test-agent"));
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

        // Verify the registry can be queried for bootstrap mode
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
}
