//! Tools section builder for the system prompt.
//!
//! Owns the `build_tools_section` function and its tests. Extracted from
//! `builder.rs` to keep that file under the 500-line limit.

use super::sections::Section;
use crate::tools::{PromptGenerationContext, ToolContext, ToolRegistry};

/// Build the Tools section content from a registry.
///
/// The registry's `build_tools_section` requires a [`PromptGenerationContext`]
/// (which carries the list of available tool names, the agent id, and the
/// workdir). We acquire that list via a single short-lived lock on
/// `list_descriptors`, release it, and then call the registry's
/// `build_tools_section` with the freshly-built context. This keeps locks
/// non-overlapping.
pub async fn build_tools_section(registry: &ToolRegistry, ctx: &ToolContext) -> Section {
    // 1. Independent lock: get available tool names, then drop the lock.
    let descriptors = registry.list_descriptors(ctx).await;
    let available_tool_names: Vec<String> = descriptors.into_iter().map(|d| d.name).collect();

    // 2. Build the prompt-generation context from the names + the existing
    //    execution context.
    let prompt_ctx = PromptGenerationContext {
        agent_id: ctx.agent_id.clone(),
        workdir: ctx.workdir.clone(),
        available_tool_names,
    };

    // 3. Acquire the registry lock again and render the section.
    let content = registry.build_tools_section(&prompt_ctx).await;
    Section::ToolsSection(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::engine::engine_eval::PermissionEngine;
    use crate::permission::rules::RuleSetBuilder;
    use crate::skills::DiskSkillRegistry;
    use crate::tools::builtin::register_builtin_tools;
    use std::sync::Arc;

    fn test_permission_engine() -> Arc<PermissionEngine> {
        Arc::new(PermissionEngine::new_with_default_data_root(
            RuleSetBuilder::new().build().unwrap(),
        ))
    }

    #[tokio::test]
    async fn test_build_tools_section_returns_tools_section() {
        let registry = ToolRegistry::new();
        let disk_registry = Arc::new(DiskSkillRegistry::new(vec![]));
        register_builtin_tools(&registry, disk_registry, test_permission_engine()).await;
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
        register_builtin_tools(&registry, disk_registry, test_permission_engine()).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
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
        register_builtin_tools(&registry, disk_registry, test_permission_engine()).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
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
        register_builtin_tools(&registry, disk_registry, test_permission_engine()).await;
        let ctx = crate::tools::ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        };
        let section = build_tools_section(&registry, &ctx).await;
        let content = match section {
            Section::ToolsSection(c) => c,
            _ => panic!("expected ToolsSection"),
        };
        assert!(
            content.chars().count() <= 15000,
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
        assert!(
            content.is_empty(),
            "expected empty content, got: {}",
            content
        );
    }
}
