//! ToolRegistry —并发安全的工具注册中心
//!
//! 支持注册、查询、列表操作，内部使用 `tokio::sync::RwLock` 保证并发安全。

use std::sync::Arc;

use crate::tools::{Tool, ToolContext, ToolDescriptor, ToolError};

/// Maximum length of the first-level tools section (in characters).
const TOOLS_SECTION_MAX_LEN: usize = 1500;

/// Thread-safe tool registry.
///
/// Wraps an inner `HashMap<String, Arc<dyn Tool>>` behind a Tokio
/// read-write lock so that all operations are async-safe.
pub struct ToolRegistry {
    tools: tokio::sync::RwLock<std::collections::HashMap<String, Arc<dyn Tool>>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry").finish()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Registers a tool.
    ///
    /// # Errors
    /// Returns [`ToolError::AlreadyRegistered`] if a tool with the same name
    /// is already present.
    pub async fn register<T: Tool + 'static>(&self, tool: T) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        let mut guard = self.tools.write().await;
        if guard.contains_key(&name) {
            return Err(ToolError::AlreadyRegistered(name));
        }
        guard.insert(name, Arc::new(tool));
        Ok(())
    }

    /// Returns all registered tool descriptors, filtered by `ctx`.
    pub async fn list_descriptors(&self, _ctx: &ToolContext) -> Vec<ToolDescriptor> {
        let guard = self.tools.read().await;
        guard
            .values()
            .map(|t| ToolDescriptor {
                name: t.name().to_string(),
                group: t.group().to_string(),
                summary: t.summary(),
                is_deferred: t.flags().is_deferred_by_default,
            })
            .collect()
    }

    /// Returns the detail string for a named tool.
    ///
    /// # Errors
    /// Returns [`ToolError::NotFound`] if no tool with that name exists.
    pub async fn get_detail(&self, name: &str) -> Result<String, ToolError> {
        let guard = self.tools.read().await;
        guard
            .get(name)
            .map(|t| t.detail())
            .ok_or_else(|| ToolError::NotFound(name.to_string()))
    }

    /// Returns all tool names belonging to a given group.
    pub async fn list_by_group(&self, group: &str) -> Vec<String> {
        let guard = self.tools.read().await;
        guard
            .values()
            .filter(|t| t.group() == group)
            .map(|t| t.name().to_string())
            .collect()
    }

    /// Returns the inner lock for read operations (used in tests only).
    #[cfg(test)]
    pub async fn len_for_test(&self) -> usize {
        self.tools.read().await.len()
    }

    /// Build a first-level tools section string, grouped and truncated.
    ///
    /// Groups tools by `group()`, formats each group with a header and tool
    /// list, then truncates at `TOOLS_SECTION_MAX_LEN` if needed.
    pub async fn build_tools_section(&self, ctx: &ToolContext) -> String {
        let descriptors = self.list_descriptors(ctx).await;

        // Group by group name
        let mut groups_map: std::collections::HashMap<String, Vec<(String, bool)>> =
            std::collections::HashMap::new();
        for d in descriptors {
            groups_map
                .entry(d.group)
                .or_default()
                .push((d.name, d.is_deferred));
        }

        // Build lines
        let mut lines: Vec<String> = Vec::new();
        let mut total_len = 0;

        let mut sorted_groups: Vec<_> = groups_map.into_iter().collect();
        sorted_groups.sort_by_key(|(g, _)| g.clone());

        for (group_name, mut tools) in sorted_groups {
            let tag = if tools.iter().any(|(_, deferred)| !deferred) {
                "(always loaded)"
            } else {
                ""
            };
            let header = if tag.is_empty() {
                format!("**{}**", group_name)
            } else {
                format!("**{}** — {}", group_name, tag)
            };

            tools.sort_by_key(|(n, _)| n.clone());
            let tool_names: Vec<_> = tools.iter().map(|(n, _)| n.clone()).collect();
            let tool_list = tool_names.join("、");
            let line = format!("{}\n- {}\n", header, tool_list);
            total_len += line.chars().count();

            if total_len > TOOLS_SECTION_MAX_LEN {
                let overflow = total_len - TOOLS_SECTION_MAX_LEN;
                let exceeded = format!(
                    "... ({} more tools, use ToolSearch to explore)",
                    overflow / 10 + 1
                );
                lines.push(exceeded);
                break;
            }

            lines.push(line);
        }

        lines.join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolFlags;

    struct DummyTool {
        name: String,
        group: String,
        summary_text: String,
        is_deferred: bool,
    }

    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn group(&self) -> &str {
            &self.group
        }
        fn summary(&self) -> String {
            self.summary_text.clone()
        }
        fn detail(&self) -> String {
            format!("detail for {}", self.name)
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        fn flags(&self) -> ToolFlags {
            let mut f = ToolFlags::default();
            f.is_deferred_by_default = self.is_deferred;
            f
        }
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".to_string(),
            workdir: None,
        }
    }

    #[tokio::test]
    async fn test_register_and_get_detail() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read file contents".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();

        let detail = reg.get_detail("Read").await.unwrap();
        assert!(detail.contains("Read"));
    }

    #[tokio::test]
    async fn test_register_not_found() {
        let reg = ToolRegistry::new();
        let err = reg.get_detail("NonExistent").await.unwrap_err();
        assert!(matches!(err, ToolError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_register_duplicate() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();

        let err = reg
            .register(DummyTool {
                name: "Read".to_string(),
                group: "file_ops".to_string(),
                summary_text: "Read again".to_string(),
                is_deferred: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::AlreadyRegistered(_)));
    }

    #[tokio::test]
    async fn test_list_descriptors() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read files".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();
        reg.register(DummyTool {
            name: "Write".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Write files".to_string(),
            is_deferred: true,
        })
        .await
        .unwrap();

        let ctx = make_ctx();
        let descriptors = reg.list_descriptors(&ctx).await;
        assert_eq!(descriptors.len(), 2);
        let read_desc = descriptors.iter().find(|d| d.name == "Read").unwrap();
        assert_eq!(read_desc.group, "file_ops");
        assert!(!read_desc.is_deferred);
        let write_desc = descriptors.iter().find(|d| d.name == "Write").unwrap();
        assert!(write_desc.is_deferred);
    }

    #[tokio::test]
    async fn test_list_by_group() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "R".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();
        reg.register(DummyTool {
            name: "ToolSearch".to_string(),
            group: "meta".to_string(),
            summary_text: "T".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();

        let file_ops = reg.list_by_group("file_ops").await;
        assert_eq!(file_ops, vec!["Read"]);

        let meta = reg.list_by_group("meta").await;
        assert_eq!(meta, vec!["ToolSearch"]);
    }

    #[tokio::test]
    async fn test_list_by_group_empty() {
        let reg = ToolRegistry::new();
        let result = reg.list_by_group("nonexistent").await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_build_tools_section() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read files".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();
        reg.register(DummyTool {
            name: "ToolSearch".to_string(),
            group: "meta".to_string(),
            summary_text: "Search tools".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();

        let ctx = make_ctx();
        let section = reg.build_tools_section(&ctx).await;
        assert!(section.contains("file_ops"));
        assert!(section.contains("Read"));
        assert!(section.contains("meta"));
        assert!(section.contains("ToolSearch"));
    }

    #[tokio::test]
    async fn test_build_tools_section_empty() {
        let reg = ToolRegistry::new();
        let ctx = make_ctx();
        let section = reg.build_tools_section(&ctx).await;
        assert!(section.is_empty());
    }
}
