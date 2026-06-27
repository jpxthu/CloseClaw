//! Tool registry trait for decoupling gateway from concrete tool implementation.
//!
//! Provides an interface for querying available tools without requiring
//! a direct dependency on the tools crate.

use async_trait::async_trait;
use serde_json::Value;

/// Tool runtime flags — controls tool behavior in the execution context.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolFlags {
    /// Tool is safe to call concurrently from multiple agents.
    pub is_concurrency_safe: bool,
    /// Tool only reads data, never modifies files or state.
    pub is_read_only: bool,
    /// Tool may overwrite or delete data — requires explicit confirmation.
    pub is_destructive: bool,
    /// Tool may be slow or consume significant resources.
    pub is_expensive: bool,
    /// Tool detail is NOT loaded into system prompt by default.
    pub is_deferred_by_default: bool,
}

impl ToolFlags {
    /// Returns true if the tool should be loaded into the system prompt
    /// by default (i.e., NOT deferred).
    #[inline]
    pub fn is_eager(&self) -> bool {
        !self.is_deferred_by_default
    }
}

/// Summary information about a tool for system prompt generation.
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    /// Tool name (unique identifier).
    pub name: String,
    /// Tool group (e.g. "file", "search", "session").
    pub group: String,
    /// Brief description for the system prompt.
    pub detail: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
    /// Runtime flags.
    pub flags: ToolFlags,
}

/// Trait for querying and managing tools.
///
/// Implemented by `ToolRegistry` in the tools crate; used by the gateway's
/// session manager and system prompt builder to list available tools
/// without a direct dependency on the tools module.
#[async_trait]
pub trait ToolRegistryQuery: Send + Sync {
    /// List all registered tool names.
    async fn list_tool_names(&self) -> Vec<String>;

    /// Get tool descriptors for system prompt generation.
    ///
    /// Returns tools filtered by the agent's tool whitelist/blacklist.
    async fn get_tool_descriptors(
        &self,
        agent_id: Option<&str>,
        agent_tools: Option<&[String]>,
        agent_disallowed_tools: Option<&[String]>,
    ) -> Vec<ToolDescriptor>;

    /// Check if a tool with the given name exists.
    async fn has_tool(&self, name: &str) -> bool;

    /// Get the JSON Schema for a tool's input parameters.
    async fn get_tool_schema(&self, name: &str) -> Option<Value>;
}
