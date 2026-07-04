//! ToolRegistry —并发安全的工具注册中心
//!
//! 支持注册、查询、列表操作，内部使用 `tokio::sync::RwLock` 保证并发安全。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use crate::{PromptGenerationContext, Tool, ToolContext, ToolDescriptor, ToolError};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

// Re-export for tests that `use super::*`
pub use ToolRegistryImpl as ToolRegistry;

/// Wrapper to bridge [`Tool`] with [`std::any::Any`] for type-erased registration.
pub(crate) struct ToolBox(pub Arc<dyn Tool>);

use closeclaw_common::AgentToolsConfigQuery;
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
pub struct ToolRegistryImpl {
    pub(crate) tools: tokio::sync::RwLock<std::collections::HashMap<String, Arc<dyn Tool>>>,
    /// Maps tool name to the registrar that registered it (for conflict reporting).
    pub(crate) owners: tokio::sync::RwLock<std::collections::HashMap<String, String>>,
    /// Optional reference to an agent tools config query for direct config queries.
    agent_tools_query: OnceLock<Arc<dyn AgentToolsConfigQuery>>,
    /// When `true`, no further registrations are accepted.
    frozen: AtomicBool,
}

impl std::fmt::Debug for ToolRegistryImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistryImpl").finish()
    }
}

impl Default for ToolRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistryImpl {
    /// Set the AgentToolsConfigQuery reference for direct config queries.
    ///
    /// Called during daemon initialization after the AgentRegistry is created.
    /// Panics if called more than once.
    pub fn set_agent_tools_query(&self, query: Arc<dyn AgentToolsConfigQuery>) {
        if self.agent_tools_query.set(query).is_err() {
            panic!("AgentToolsConfigQuery already set on ToolRegistry");
        }
    }

    /// Get the current AgentToolsConfigQuery reference, if set.
    pub fn get_agent_tools_query(&self) -> Option<&Arc<dyn AgentToolsConfigQuery>> {
        self.agent_tools_query.get()
    }

    /// Query agent-level tool filtering configuration.
    ///
    /// Returns `(tools, disallowed_tools)` extracted from the agent's config.
    /// When the agent is not found or no query is set, returns `(None, None)`
    /// (no filtering — all tools allowed).
    pub async fn query_agent_tools_config(
        &self,
        agent_id: &str,
    ) -> (Option<Vec<String>>, Option<Vec<String>>) {
        let Some(query) = self.agent_tools_query.get() else {
            return (None, None);
        };
        let Some(config) = query.get_agent_tools_config(agent_id).await else {
            return (None, None);
        };
        (config.tools, config.disallowed_tools)
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

impl ToolRegistryImpl {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            owners: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            agent_tools_query: OnceLock::new(),
            frozen: AtomicBool::new(false),
        }
    }

    /// Registers a tool.
    ///
    /// # Errors
    /// Returns [`ToolError::AlreadyRegistered`] if a tool with the same name
    /// is already present.
    pub async fn register<T: Tool + 'static>(&self, tool: T) -> Result<(), ToolError> {
        if self.frozen.load(Ordering::Acquire) {
            return Err(ToolError::Frozen);
        }
        let name = tool.name().to_string();
        let mut guard = self.tools.write().await;
        if guard.contains_key(&name) {
            return Err(ToolError::AlreadyRegistered(name));
        }
        guard.insert(name, Arc::new(tool));
        Ok(())
    }

    /// Register all tools from the given registrars, sorted by priority.
    ///
    /// After all registrars have been called successfully, the registry is
    /// frozen — subsequent calls to [`register`](Self::register) will return
    /// [`ToolError::Frozen`].
    ///
    /// # Errors
    /// Returns [`ToolRegistrarError::Conflict`] if a tool name collision is
    /// detected between registrars.
    pub async fn register_all(
        &self,
        registrars: Vec<Box<dyn ToolRegistrar>>,
    ) -> Result<(), ToolRegistrarError> {
        if self.frozen.load(Ordering::Acquire) {
            return Err(ToolRegistrarError::Internal(
                "registry is already frozen".to_string(),
            ));
        }

        let mut sorted = registrars;
        sorted.sort_by_key(|r| r.priority());

        for registrar in &sorted {
            registrar
                .register(self as &dyn closeclaw_common::tool_registry::ToolRegistry)
                .await?;
        }

        self.frozen.store(true, Ordering::Release);
        Ok(())
    }

    /// Build a first-level tools index string without a prompt generation context.
    ///
    /// Groups tools by group, shows name + detail for eager tools,
    /// name + danger marks for deferred tools.
    pub async fn build_index_raw(&self) -> String {
        let guard = self.tools.read().await;
        let tool_infos: Vec<ToolInfo> = guard
            .values()
            .map(|t| {
                ToolInfo::from_tool(
                    t,
                    &crate::PromptGenerationContext {
                        agent_id: String::new(),
                        workdir: None,
                        available_tool_names: Vec::new(),
                        tools: None,
                        disallowed_tools: None,
                    },
                )
            })
            .collect();

        let mut groups_map: std::collections::HashMap<String, Vec<ToolInfo>> =
            std::collections::HashMap::new();
        for info in tool_infos {
            groups_map.entry(info.group.clone()).or_default().push(info);
        }

        let mut lines: Vec<String> = Vec::new();
        let mut sorted_groups: Vec<_> = groups_map.into_iter().collect();
        sorted_groups.sort_by_key(|(g, _)| g.clone());

        for (group_name, tools) in sorted_groups {
            let has_eager = tools.iter().any(|t| !t.is_deferred);
            let tag = if has_eager {
                "(always loaded)"
            } else {
                "(deferred)"
            };
            lines.push(format!("**{}** — {}", group_name, tag));
            let mut sorted_tools: Vec<_> = tools.iter().collect();
            sorted_tools.sort_by_key(|t| t.name.clone());
            for tool in sorted_tools {
                let danger_mark = if tool.is_destructive {
                    " (destructive)"
                } else if tool.is_read_only {
                    " (read-only)"
                } else {
                    ""
                };
                if tool.is_deferred {
                    lines.push(format!("  - {}{}", tool.name, danger_mark));
                } else {
                    lines.push(format!(
                        "  - **{}**{}: {}",
                        tool.name, danger_mark, tool.detail
                    ));
                }
            }
            lines.push(String::new());
        }
        lines.join("\n")
    }

    /// Returns whether the registry is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen.load(Ordering::Acquire)
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

    /// Returns the number of registered tools.
    pub async fn len_for_test(&self) -> usize {
        self.tools.read().await.len()
    }
}

impl ToolRegistryImpl {
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

// ═══════════════════════════════════════════════════════════════════════════
// ToolRegistryQuery — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_common::tool_registry::ToolRegistryQuery for ToolRegistryImpl {
    async fn list_tool_names(&self) -> Vec<String> {
        let guard = self.tools.read().await;
        guard.keys().cloned().collect()
    }

    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        agent_tools: Option<&[String]>,
        agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<closeclaw_common::tool_registry::ToolDescriptor> {
        let guard = self.tools.read().await;
        let allowed: Option<&[String]> =
            agent_tools.filter(|t| !(t.is_empty() || t.len() == 1 && t[0] == "*"));
        let disallowed: &[String] = agent_disallowed_tools.unwrap_or(&[]);

        guard
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
            .map(|t| {
                let flags = t.flags();
                closeclaw_common::tool_registry::ToolDescriptor {
                    name: t.name().to_string(),
                    group: t.group().to_string(),
                    detail: t.detail(),
                    input_schema: t.input_schema(),
                    flags: closeclaw_common::tool_registry::ToolFlags {
                        is_concurrency_safe: flags.is_concurrency_safe,
                        is_read_only: flags.is_read_only,
                        is_destructive: flags.is_destructive,
                        is_expensive: flags.is_expensive,
                        is_deferred_by_default: flags.is_deferred_by_default,
                    },
                }
            })
            .collect()
    }

    async fn has_tool(&self, name: &str) -> bool {
        let guard = self.tools.read().await;
        guard.contains_key(name)
    }

    async fn get_tool_schema(&self, name: &str) -> Option<serde_json::Value> {
        let guard = self.tools.read().await;
        guard.get(name).map(|t| t.input_schema())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ToolRegistry — bridge to closeclaw_common trait
// ═══════════════════════════════════════════════════════════════════════════

#[async_trait::async_trait]
impl closeclaw_common::tool_registry::ToolRegistry for ToolRegistryImpl {
    async fn build_index(&self) -> String {
        self.build_index_raw().await
    }
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), closeclaw_common::tool_registry::RegistryError> {
        let ToolBox(arc_tool) = *tool.downcast::<ToolBox>().map_err(|_| {
            closeclaw_common::tool_registry::RegistryError::Internal(
                "register_any expected ToolBox".to_string(),
            )
        })?;
        let name = (*arc_tool).name().to_string();
        if self.frozen.load(Ordering::Acquire) {
            return Err(closeclaw_common::tool_registry::RegistryError::Frozen);
        }
        let mut guard = self.tools.write().await;
        if guard.contains_key(&name) {
            let owners = self.owners.read().await;
            let original = owners.get(&name).cloned().unwrap_or_default();
            drop(guard);
            drop(owners);
            return Err(closeclaw_common::tool_registry::RegistryError::Conflict {
                tool: name,
                registrar: original,
                attempting: registrar_name.to_string(),
            });
        }
        guard.insert(name.clone(), arc_tool);
        drop(guard);
        let mut owners = self.owners.write().await;
        owners.insert(name, registrar_name.to_string());
        Ok(())
    }

    fn freeze(&self) {
        self.frozen.store(true, Ordering::Release);
    }

    fn is_frozen(&self) -> bool {
        self.frozen.load(Ordering::Acquire)
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
