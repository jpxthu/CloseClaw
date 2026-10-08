//! In-memory [`ToolRegistry`] stand-in for im_adapter's own tests.
//!
//! The concrete registry implementation lives in the tools crate, which
//! `im_adapter` must not depend on (STANDARDS.md dependency allow-list).
//! Tests therefore register against this local implementation, which is
//! built purely on the `closeclaw-common` traits and preserves the two
//! behaviours the registrar tests assert: name-conflict rejection and
//! descriptor listing.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::tool_registry::{
    RegistryError, ToolBox, ToolDescriptor, ToolRegistry, ToolRegistryQuery,
};
use closeclaw_common::tool_trait::{Tool, ToolCallError, ToolContext, ToolResult};
use tokio::sync::RwLock;

/// Summary row returned by [`TestToolRegistry::list_descriptors`].
pub struct ToolSummary {
    /// Registered tool name.
    pub name: String,
    /// Registered tool group.
    pub group: String,
    /// Whether the tool is deferred by default.
    pub is_deferred: bool,
}

/// A registered tool plus the registrar that admitted it.
struct Entry {
    tool: Arc<dyn Tool>,
    registrar: String,
}

/// Minimal registry implementing the `closeclaw-common` registry traits.
pub struct TestToolRegistry {
    tools: RwLock<BTreeMap<String, Entry>>,
    frozen: AtomicBool,
}

impl TestToolRegistry {
    /// Create an empty, unfrozen registry.
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(BTreeMap::new()),
            frozen: AtomicBool::new(false),
        }
    }

    /// Returns all registered tool summaries.
    ///
    /// `ctx` is accepted for signature parity with the production registry
    /// and is intentionally unused.
    pub async fn list_descriptors(&self, _ctx: &ToolContext) -> Vec<ToolSummary> {
        self.tools
            .read()
            .await
            .values()
            .map(|entry| ToolSummary {
                name: entry.tool.name().to_string(),
                group: entry.tool.group().to_string(),
                is_deferred: entry.tool.flags().is_deferred_by_default,
            })
            .collect()
    }

    /// Returns the number of registered tools.
    pub async fn len_for_test(&self) -> usize {
        self.tools.read().await.len()
    }

    async fn descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        let guard = self.tools.read().await;
        let tool = &guard.get(name)?.tool;
        Some(ToolDescriptor {
            name: tool.name().to_string(),
            group: tool.group().to_string(),
            summary: tool.summary(),
            detail: tool.detail(),
            input_schema: tool.input_schema(),
            flags: tool.flags(),
        })
    }
}

impl Default for TestToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolRegistryQuery for TestToolRegistry {
    async fn list_tool_names(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    async fn get_tool_descriptors(
        &self,
        _agent_id: Option<&str>,
        _agent_tools: Option<&[String]>,
        _agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor> {
        let names: Vec<String> = self.list_tool_names().await;
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if let Some(descriptor) = self.descriptor(&name).await {
                out.push(descriptor);
            }
        }
        out
    }

    async fn has_tool(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    async fn get_tool_schema(&self, name: &str) -> Option<serde_json::Value> {
        let guard = self.tools.read().await;
        guard.get(name).map(|entry| entry.tool.input_schema())
    }

    async fn get_tool_detail(&self, name: &str) -> Option<ToolDescriptor> {
        self.descriptor(name).await
    }

    async fn list_tool_names_by_group(&self, group: &str) -> Vec<String> {
        self.tools
            .read()
            .await
            .values()
            .filter(|entry| entry.tool.group() == group)
            .map(|entry| entry.tool.name().to_string())
            .collect()
    }

    async fn get_tool_concurrency_safe(&self, name: &str) -> Option<bool> {
        let guard = self.tools.read().await;
        guard
            .get(name)
            .map(|entry| entry.tool.flags().is_concurrency_safe)
    }

    async fn call_tool(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolCallError> {
        let tool = {
            let guard = self.tools.read().await;
            guard.get(name).map(|entry| Arc::clone(&entry.tool))
        };
        match tool {
            Some(tool) => tool.call(args, ctx).await,
            None => Err(ToolCallError::NotFound(name.to_string())),
        }
    }
}

#[async_trait]
impl ToolRegistry for TestToolRegistry {
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), RegistryError> {
        if self.frozen.load(Ordering::Acquire) {
            return Err(RegistryError::Frozen);
        }
        let tool_box = tool
            .downcast::<ToolBox>()
            .map_err(|_| RegistryError::Internal("registration payload is not a ToolBox".into()))?;
        let tool = tool_box.0;
        let name = tool.name().to_string();
        let mut guard = self.tools.write().await;
        if let Some(existing) = guard.get(&name) {
            return Err(RegistryError::Conflict {
                tool: name,
                registrar: existing.registrar.clone(),
                attempting: registrar_name.to_string(),
            });
        }
        guard.insert(
            name,
            Entry {
                tool,
                registrar: registrar_name.to_string(),
            },
        );
        Ok(())
    }

    fn freeze(&self) {
        self.frozen.store(true, Ordering::Release);
    }

    fn is_frozen(&self) -> bool {
        self.frozen.load(Ordering::Acquire)
    }

    async fn build_index(&self) -> String {
        self.tools
            .read()
            .await
            .values()
            .map(|entry| format!("{} ({})", entry.tool.name(), entry.tool.group()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
