//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use super::sections::{get_cached_section, load_cached_file_section, read_file_section, Section};
use super::tools_section::build_tools_section;
use crate::skills::DiskSkillRegistry;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Overrides for the three-tier priority prompt system.
///
/// When resolving the final system prompt, the caller (typically
/// `build_full_system_prompt`) checks these in order:
///   1. `override_prompt` — highest priority, replaces the entire static layer
///   2. `agent_prompt`    — agent-level prompt
///   3. `custom_prompt`   — user-defined custom prompt
///
/// If none is set, the normal section-based rendering is used.
#[derive(Debug, Clone, Default)]
pub struct PromptOverrides {
    pub override_prompt: Option<String>,
    pub agent_prompt: Option<String>,
    pub custom_prompt: Option<String>,
}

/// Default system prompt fallback
const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Build the complete system prompt from the given sections.
///
/// This function only renders sections and appends the optional `append_section`.
/// Priority-prompt resolution (override > agent > custom) is handled at the
/// request stage by `build_full_system_prompt` in `gateway::system_prompt_inject`.
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
                    super::sections::put_cached_section(name, rendered.clone(), None);
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

use crate::tools::{ToolContext, ToolRegistry};

// ---------------------------------------------------------------------------
// Convenience: build from file-based workspace sections
// ---------------------------------------------------------------------------

/// Configuration for `build_from_workspace`.
pub struct WorkspaceBuildConfig<'a> {
    /// Bootstrap files as (filename, content) pairs, in display order.
    /// Provided by `load_bootstrap_files`.
    pub bootstrap_files: Vec<(String, String)>,
    /// Tool registry for generating the ToolsSection.
    pub tool_registry: Option<&'a ToolRegistry>,
    /// Tool context for tool section rendering.
    pub tool_ctx: &'a ToolContext,
    /// Skill registry for generating SkillListingSection.
    pub skill_registry: Option<Arc<RwLock<Option<DiskSkillRegistry>>>>,
    /// Agent ID for skill listing filtering.
    pub agent_id: Option<&'a str>,
    /// Additional dynamic sections to include.
    pub dynamic_sections: Vec<Section>,
    /// Content to append at the end of the prompt.
    pub append_section: Option<String>,
}

/// Build a system prompt from a workspace directory.
pub async fn build_from_workspace<P: AsRef<Path>>(
    workspace_root: P,
    config: WorkspaceBuildConfig<'_>,
) -> String {
    let root = workspace_root.as_ref();
    let mut sections: Vec<Section> = Vec::new();

    // RoleSection from bootstrap files (skip MEMORY.md)
    let role: String = config
        .bootstrap_files
        .iter()
        .filter(|(n, _)| n != "MEMORY.md")
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !role.is_empty() {
        sections.push(Section::RoleSection(role));
    }
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
        sections.push(build_tools_section(reg, config.tool_ctx).await);
    } else {
        sections.push(Section::ToolsSection(String::new()));
    }
    // SkillListingSection
    if let Some(lock) = config.skill_registry {
        if let Ok(g) = lock.read() {
            if let Some(reg) = g.as_ref() {
                let listing = reg.generate_listing(config.agent_id);
                if !listing.is_empty() {
                    sections.push(Section::SkillListingSection(listing));
                }
            }
        }
    }
    sections.extend(config.dynamic_sections);
    build_system_prompt(sections, config.append_section)
}

#[cfg(test)]
mod tests {
    use super::super::sections::Section;
    use super::*;

    /// Clear cached sections to prevent cross-test pollution.
    #[cfg(test)]
    use crate::system_prompt::sections::invalidate_all_sections;

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
            turn_count: 1,
            pending_tasks: vec![],
        }];
        let result1 = build_system_prompt(sections.clone(), None);
        let result2 = build_system_prompt(sections, None);
        assert_eq!(result1, result2);
    }
}
