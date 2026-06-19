//! ToolRegistry —并发安全的工具注册中心
//!
//! 支持注册、查询、列表操作，内部使用 `tokio::sync::RwLock` 保证并发安全。

use std::sync::Arc;

use crate::agent::registry::AgentRegistry;
use crate::tools::{PromptGenerationContext, Tool, ToolContext, ToolDescriptor, ToolError};

use serde_json::Value;

/// Internal tool info carrier for `build_tools_section`.
struct ToolInfo {
    name: String,
    group: String,
    detail: String,
    #[allow(dead_code)]
    input_schema: Value,
    is_deferred: bool,
    is_read_only: bool,
    is_destructive: bool,
    #[allow(dead_code)]
    is_expensive: bool,
}

impl ToolInfo {
    fn from_tool(tool: &Arc<dyn Tool>, context: &PromptGenerationContext) -> Self {
        let flags = tool.flags();
        Self {
            name: tool.name().to_string(),
            group: tool.group().to_string(),
            // Use the dynamic Prompt layer (default falls back to `detail()`).
            detail: tool.generate_prompt(context),
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
    /// Optional reference to the AgentRegistry for direct config queries.
    ///
    /// When set, allows querying agent-level tool filtering configuration
    /// directly from the AgentRegistry, fulfilling the design-doc query path:
    ///   Tools Registry → AgentRegistry.get(agent_id) → tools / disallowed_tools
    agent_registry: tokio::sync::RwLock<Option<Arc<AgentRegistry>>>,
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
    /// Set the AgentRegistry reference for direct config queries.
    ///
    /// Called during daemon initialization after the AgentRegistry is created.
    pub async fn set_agent_registry(&self, registry: Arc<AgentRegistry>) {
        *self.agent_registry.write().await = Some(registry);
    }

    /// Get the current AgentRegistry reference, if set.
    pub async fn get_agent_registry(&self) -> Option<Arc<AgentRegistry>> {
        self.agent_registry.read().await.clone()
    }

    /// Query agent-level tool filtering configuration from the AgentRegistry.
    ///
    /// Returns `(tools, disallowed_tools)` extracted from the agent's
    /// `ResolvedAgentConfig`. When the agent is not found or the AgentRegistry
    /// is not set, returns `(None, None)` (no filtering — all tools allowed).
    pub async fn query_agent_tools_config(
        &self,
        agent_id: &str,
    ) -> (Option<Vec<String>>, Option<Vec<String>>) {
        let guard = self.agent_registry.read().await;
        let Some(registry) = guard.as_ref() else {
            return (None, None);
        };
        let Some(config) = registry.get(agent_id) else {
            return (None, None);
        };
        let tools = if config.tools.is_empty() || config.tools == ["*"] {
            None
        } else {
            Some(config.tools.clone())
        };
        let disallowed_tools = if config.disallowed_tools.is_empty() {
            None
        } else {
            Some(config.disallowed_tools.clone())
        };
        (tools, disallowed_tools)
    }

    /// Format a single group into a section line, returning (output, new_total_len).
    /// Returns None if truncation was triggered.
    ///
    /// Output format:
    /// - group header: `**{group}** — (always loaded)` if the group has eager tools, else `**{group}** — (deferred)`
    /// - eager tools: `  - **{name}**{(read-only)}: {detail}` or `  - **{name}**{(destructive)}: {detail}`
    /// - deferred tools: `  - {name}{(read-only)}` or `  - {name}{(destructive)}`
    fn format_group_line(
        group_name: &str,
        tools: &[ToolInfo],
        total_len: usize,
        max_len: usize,
    ) -> Option<(String, usize)> {
        let has_eager = tools.iter().any(|t| !t.is_deferred);
        let tag = if has_eager {
            "(always loaded)"
        } else {
            "(deferred)"
        };
        let header = format!("**{}** — {}", group_name, tag);

        let mut sorted_tools: Vec<_> = tools.iter().collect();
        sorted_tools.sort_by_key(|t| t.name.clone());

        let mut lines = vec![header];
        for tool in sorted_tools {
            let danger_mark = if tool.is_destructive {
                " (destructive)"
            } else if tool.is_read_only {
                " (read-only)"
            } else {
                ""
            };
            let line = if tool.is_deferred {
                format!("  - {}{}", tool.name, danger_mark)
            } else {
                format!("  - **{}**{}: {}", tool.name, danger_mark, tool.detail)
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
            agent_registry: tokio::sync::RwLock::new(None),
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
    ///
    /// `ctx` is the prompt-generation context: it carries agent identity,
    /// workdir, and the list of currently available tool names. Each tool's
    /// `detail` field is produced by [`Tool::generate_prompt`], so a tool that
    /// has opted into a custom Prompt layer will see a tailored description
    /// here; otherwise the static `detail()` string is used.
    pub async fn build_tools_section(&self, ctx: &PromptGenerationContext) -> String {
        let guard = self.tools.read().await;

        // Determine the set of allowed tool names.
        // Whitelist: tools == ["*"] or None → all allowed.
        // Blacklist: disallowed_tools → excluded after whitelist.
        let allowed: Option<&[String]> = ctx
            .tools
            .as_deref()
            .filter(|t| t != &["*"] && !t.is_empty());
        let disallowed: &[String] = ctx.disallowed_tools.as_deref().unwrap_or(&[]);

        // Collect ToolInfo from registered tools, filtered by
        // the agent's tools / disallowed_tools config.
        let tool_infos: Vec<ToolInfo> = guard
            .values()
            .filter(|t| {
                let name = t.name();
                if let Some(wl) = allowed {
                    if !wl.iter().any(|n| n == name) {
                        return false;
                    }
                }
                !disallowed.iter().any(|n| n == name)
            })
            .map(|t| ToolInfo::from_tool(t, ctx))
            .collect();

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
#[path = "registry_tests.rs"]
mod tests;
