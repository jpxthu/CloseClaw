//! ToolRegistry —并发安全的工具注册中心
//!
//! 支持注册、查询、列表操作，内部使用 `tokio::sync::RwLock` 保证并发安全。

use std::sync::Arc;

use crate::tools::{Tool, ToolContext, ToolDescriptor, ToolError};

use serde_json::Value;

/// Internal tool info carrier for `build_tools_section`.
struct ToolInfo {
    name: String,
    group: String,
    detail: String,
    #[allow(dead_code)]
    input_schema: Value,
    is_deferred: bool,
    #[allow(dead_code)]
    is_read_only: bool,
    #[allow(dead_code)]
    is_destructive: bool,
    #[allow(dead_code)]
    is_expensive: bool,
}

impl ToolInfo {
    fn from_tool(tool: &Arc<dyn Tool>) -> Self {
        let flags = tool.flags();
        Self {
            name: tool.name().to_string(),
            group: tool.group().to_string(),
            detail: tool.detail(),
            input_schema: tool.input_schema(),
            is_deferred: flags.is_deferred_by_default,
            is_read_only: flags.is_read_only,
            is_destructive: flags.is_destructive,
            is_expensive: flags.is_expensive,
        }
    }
}

/// Maximum length of the first-level tools section (in characters).
const TOOLS_SECTION_MAX_LEN: usize = 15000;

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
    /// Format a single group into a section line, returning (output, new_total_len).
    /// Returns None if truncation was triggered.
    ///
    /// Output format:
    /// - group header: `**{group}** — (always loaded)` if the group has eager tools
    /// - eager tools: `  - **{name}**: {detail}`
    /// - deferred tools: `  - {name}`
    fn format_group_line(
        group_name: &str,
        tools: &[ToolInfo],
        total_len: usize,
        max_len: usize,
    ) -> Option<(String, usize)> {
        let has_eager = tools.iter().any(|t| !t.is_deferred);
        let tag = if has_eager { "(always loaded)" } else { "" };
        let header = if tag.is_empty() {
            format!("**{}**", group_name)
        } else {
            format!("**{}** — {}", group_name, tag)
        };

        let mut sorted_tools: Vec<_> = tools.iter().collect();
        sorted_tools.sort_by_key(|t| t.name.clone());

        let mut lines = vec![header];
        for tool in sorted_tools {
            let line = if tool.is_deferred {
                format!("  - {}", tool.name)
            } else {
                format!("  - **{}**: {}", tool.name, tool.detail)
            };
            lines.push(line);
        }

        let output = lines.join("\n") + "\n";
        let new_len = total_len + output.chars().count();
        if new_len > max_len {
            None
        } else {
            Some((output, new_len))
        }
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
}

impl ToolRegistry {
    /// Build a first-level tools section string, grouped and truncated.
    ///
    /// Groups tools by `group()`, formats each group with a header and tool
    /// list, then truncates at `TOOLS_SECTION_MAX_LEN` if needed.
    pub async fn build_tools_section(&self, _ctx: &ToolContext) -> String {
        let guard = self.tools.read().await;

        // Collect ToolInfo from all registered tools
        let tool_infos: Vec<ToolInfo> = guard.values().map(|t| ToolInfo::from_tool(t)).collect();

        // Group by group name
        let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
            std::collections::HashMap::new();
        for info in tool_infos {
            groups_map.entry(info.group.clone()).or_default().push(info);
        }

        let mut lines: Vec<String> = Vec::new();
        let mut total_len = 0;

        let mut sorted_groups: Vec<_> = groups_map.into_iter().collect();
        sorted_groups.sort_by_key(|(g, _)| g.clone());

        for (group_name, tools) in sorted_groups {
            let Some((line, new_len)) =
                Self::format_group_line(&group_name, &tools, total_len, TOOLS_SECTION_MAX_LEN)
            else {
                break;
            };
            total_len = new_len;
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
    async fn test_tool_info_from_tool() {
        let reg = ToolRegistry::new();
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read files".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();

        let guard = reg.tools.read().await;
        let tool = guard.get("Read").unwrap();
        let info = ToolInfo::from_tool(tool);
        assert_eq!(info.name, "Read");
        assert_eq!(info.group, "file_ops");
        assert_eq!(info.detail, "detail for Read");
        assert!(!info.is_deferred);
        assert!(!info.is_read_only);
        assert!(!info.is_destructive);
        assert!(!info.is_expensive);
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
        assert!(section.contains("**Read**: detail for Read"));
        assert!(section.contains("meta"));
        assert!(section.contains("**ToolSearch**: detail for ToolSearch"));
    }

    #[tokio::test]
    async fn test_build_tools_section_with_detail() {
        let reg = ToolRegistry::new();
        // Eager tool — should show detail
        reg.register(DummyTool {
            name: "Read".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Read files".to_string(),
            is_deferred: false,
        })
        .await
        .unwrap();
        // Deferred tool — should show name only
        reg.register(DummyTool {
            name: "Write".to_string(),
            group: "file_ops".to_string(),
            summary_text: "Write files".to_string(),
            is_deferred: true,
        })
        .await
        .unwrap();

        let ctx = make_ctx();
        let section = reg.build_tools_section(&ctx).await;
        // Eager: bold name + detail
        assert!(
            section.contains("**Read**: detail for Read"),
            "eager tool should show detail, got: {section}"
        );
        // Deferred: name only, no bold/detail
        assert!(
            section.contains("  - Write"),
            "deferred tool should show name only, got: {section}"
        );
        assert!(
            !section.contains("**Write**:"),
            "deferred tool should NOT have bold detail, got: {section}"
        );
    }

    #[tokio::test]
    async fn test_build_tools_section_empty() {
        let reg = ToolRegistry::new();
        let ctx = make_ctx();
        let section = reg.build_tools_section(&ctx).await;
        assert!(section.is_empty());
    }
}
