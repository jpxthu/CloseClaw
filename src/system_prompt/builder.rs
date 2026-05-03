//! System Prompt Builder
//!
//! Orchestrates section assembly and renders the final system prompt string.

use super::sections::{
    get_append_section, get_cached_section, invalidate_all_sections, invalidate_section,
    load_cached_file_section, read_file_section, Section,
};
use crate::skills::DiskSkillRegistry;
use std::path::Path;
use std::sync::RwLock;

/// Static override: if set, replaces the entire prompt
static OVERRIDE_PROMPT: RwLock<Option<String>> = RwLock::new(None);

/// Agent system prompt: loaded from agent config / workspace
static AGENT_PROMPT: RwLock<Option<String>> = RwLock::new(None);

/// Custom system prompt: from user config
static CUSTOM_PROMPT: RwLock<Option<String>> = RwLock::new(None);

/// Default system prompt fallback
const DEFAULT_PROMPT: &str = "You are CloseClaw, a helpful AI assistant.";

// ---------------------------------------------------------------------------
// Override / agent / custom prompt management
// ---------------------------------------------------------------------------

/// Set an override system prompt (takes precedence over everything)
pub fn set_override_prompt(prompt: Option<String>) {
    if let Ok(mut guard) = OVERRIDE_PROMPT.write() {
        *guard = prompt;
    }
}

/// Set the agent-level system prompt
pub fn set_agent_prompt(prompt: Option<String>) {
    if let Ok(mut guard) = AGENT_PROMPT.write() {
        *guard = prompt;
    }
}

/// Set the custom system prompt
pub fn set_custom_prompt(prompt: Option<String>) {
    if let Ok(mut guard) = CUSTOM_PROMPT.write() {
        *guard = prompt;
    }
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Build the complete system prompt from the given sections.
///
/// Priority (highest to lowest):
///  1. overrideSystemPrompt (if set)
///  2. agentSystemPrompt (if set)
///  3. CustomSystemPrompt (if set)
///  4. defaultSystemPrompt
///  5. appendSection (always appended last)
pub fn build_system_prompt(sections: Vec<Section>) -> String {
    // Check priority prompts first (early return)
    if let Some(prompt) = get_priority_prompt() {
        return append_append_section(prompt);
    }

    // Render sections
    let rendered = render_sections(sections);
    let base = if rendered.is_empty() {
        DEFAULT_PROMPT.to_string()
    } else {
        rendered.join("\n")
    };

    append_append_section(base)
}

/// Get the highest-priority prompt that is set
fn get_priority_prompt() -> Option<String> {
    // Check override
    if let Ok(guard) = OVERRIDE_PROMPT.read() {
        if let Some(ref prompt) = *guard {
            return Some(prompt.clone());
        }
    }
    // Check agent
    if let Ok(guard) = AGENT_PROMPT.read() {
        if let Some(ref prompt) = *guard {
            return Some(prompt.clone());
        }
    }
    // Check custom
    if let Ok(guard) = CUSTOM_PROMPT.read() {
        if let Some(ref prompt) = *guard {
            return Some(prompt.clone());
        }
    }
    None
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
fn append_append_section(base: String) -> String {
    if let Some(append) = get_append_section() {
        format!("{}\n\n## Append\n{}\n", base, append)
    } else {
        base
    }
}

// ---------------------------------------------------------------------------
// Tools section builder
// ---------------------------------------------------------------------------

use crate::tools::{ToolContext, ToolRegistry};

/// Build the Tools section content from a registry.
///
/// Groups tools by their group name, formats each group with a header and
/// tool list, then truncates at `TOOLS_SECTION_MAX_LEN` (1500 chars) if needed.
pub async fn build_tools_section(registry: &ToolRegistry, ctx: &ToolContext) -> Section {
    let content = registry.build_tools_section(ctx).await;
    Section::ToolsSection(content)
}

// ---------------------------------------------------------------------------
// Convenience: build from file-based workspace sections
// ---------------------------------------------------------------------------

/// Build a system prompt using standard workspace file paths.
///
/// Reads IDENTITY.md, SOUL.md, and MEMORY.md from the workspace root
/// and assembles them with the provided dynamic sections.
///
/// The `skill_info` parameter, when provided as `Some((registry, agent_id))`,
/// injects a `SkillListingSection` after the ToolsSection for skill discovery.
pub fn build_from_workspace<P: AsRef<Path>>(
    workspace_root: P,
    dynamic_sections: Vec<Section>,
    skill_info: Option<(&DiskSkillRegistry, &str)>,
) -> String {
    let root = workspace_root.as_ref();
    let mut sections: Vec<Section> = Vec::new();

    // Static sections from workspace files
    let identity_path = root.join("IDENTITY.md");
    if identity_path.exists() {
        if let Some((content, _)) = read_file_section(&identity_path) {
            sections.push(Section::RoleSection(content));
        }
    }

    let soul_path = root.join("SOUL.md");
    if soul_path.exists() {
        if let Some((content, _)) = read_file_section(&soul_path) {
            // Append to role section rather than separate
            if let Some(Section::RoleSection(ref mut existing)) = sections.last_mut() {
                existing.push_str("\n\n");
                existing.push_str(&content);
            } else {
                sections.push(Section::RoleSection(content));
            }
        }
    }

    let memory_path = root.join("MEMORY.md");
    if memory_path.exists() {
        if let Some((content, _)) = read_file_section(&memory_path) {
            sections.push(Section::MemorySection(content));
        }
    } else {
        sections.push(Section::MemorySection(String::new()));
    }

    // Tools section — inserted between RoleSection and MemorySection
    sections.push(Section::ToolsSection(String::new()));

    // Skill listing section — injected after ToolsSection, before dynamic_sections
    if let Some((registry, agent_id)) = skill_info {
        let listing = registry.generate_listing(Some(agent_id));
        if !listing.is_empty() {
            sections.push(Section::SkillListingSection(listing));
        }
    }

    // Dynamic sections
    sections.extend(dynamic_sections);

    build_system_prompt(sections)
}

#[cfg(test)]
mod tests {
    use super::super::sections::Section;
    use super::super::sections::{clear_append_section, set_append_section};
    use super::*;
    use crate::skills::DiskSkillRegistry;
    use crate::tools::builtin::register_builtin_tools;
    use crate::tools::ToolRegistry;
    use std::sync::Arc;

    #[test]
    fn test_build_system_prompt_with_override() {
        clear_append_section();
        set_override_prompt(Some("override prompt".to_string()));
        let sections = vec![Section::RoleSection("should not appear".to_string())];
        let result = build_system_prompt(sections);
        assert!(result.contains("override prompt"));
        set_override_prompt(None);
    }

    #[test]
    fn test_build_system_prompt_with_agent_prompt() {
        clear_append_section();
        set_agent_prompt(Some("agent prompt".to_string()));
        let sections = vec![];
        let result = build_system_prompt(sections);
        assert!(result.contains("agent prompt"));
        set_agent_prompt(None);
    }

    #[test]
    fn test_build_system_prompt_with_custom_prompt() {
        clear_append_section();
        set_custom_prompt(Some("custom prompt".to_string()));
        let sections = vec![];
        let result = build_system_prompt(sections);
        assert!(result.contains("custom prompt"));
        set_custom_prompt(None);
    }

    #[test]

    fn test_build_system_prompt_default() {
        // Clear global state that could affect this test
        clear_append_section();
        set_override_prompt(None);
        set_agent_prompt(None);
        set_custom_prompt(None);
        invalidate_all_sections();

        let sections = vec![Section::RoleSection("role content".to_string())];
        let result = build_system_prompt(sections);
        assert!(result.contains("role content"));
    }

    #[test]

    fn test_build_append_section_appended() {
        set_override_prompt(None);
        clear_append_section();
        set_append_section("extra notes".to_string());
        let sections = vec![Section::RoleSection("base".to_string())];
        let result = build_system_prompt(sections);
        assert!(result.contains("base"));
        assert!(result.contains("extra notes"));
        clear_append_section();
    }

    #[test]

    fn test_append_section_not_shown_when_empty() {
        clear_append_section();
        let sections = vec![Section::RoleSection("base".to_string())];
        let result = build_system_prompt(sections);
        // append section should not appear at all
        assert!(!result.contains("## Append"));
        clear_append_section();
    }

    #[test]
    fn test_dynamic_sections_not_cached() {
        clear_append_section();
        let sections = vec![Section::SessionState {
            turn_count: 1,
            pending_tasks: vec![],
        }];
        let result1 = build_system_prompt(sections.clone());
        let result2 = build_system_prompt(sections);
        assert_eq!(result1, result2);
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_tools_section() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        register_builtin_tools(&registry, disk_registry).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        match section {
            Section::ToolsSection(_) => {}
            _ => panic!("expected ToolsSection, got {:?}", section),
        }
    }

    #[tokio::test]
    async fn test_build_tools_section_contains_group_headers() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        register_builtin_tools(&registry, disk_registry).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        // Should contain file_ops and meta group headers
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
        register_builtin_tools(&registry, disk_registry).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        // All 7 tool names should appear
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
        register_builtin_tools(&registry, disk_registry).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        // With 7 tools the section should be well under 1500 chars
        assert!(
            content.chars().count() <= 1500,
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
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        // Empty registry should produce empty content
        assert!(
            content.is_empty(),
            "expected empty content, got: {}",
            content
        );
    }
}
