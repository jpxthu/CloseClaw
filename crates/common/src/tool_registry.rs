//! Core traits for tool registration and querying.
//!
//! - [`ToolRegistrar`]: modules implement this to register tools at startup.
//! - [`ToolRegistry`]: the central registry interface (register, freeze, query).
//! - [`ToolRegistryQuery`]: read-only query interface for the registry.

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::tool_trait::Tool;

/// Wrapper to bridge [`Tool`] with [`std::any::Any`] for type-erased registration.
///
/// Used by [`register_single`] and the tools crate's [`ToolRegistry`]
/// to convert concrete tool types into `Box<dyn Any + Send + Sync>`.
pub struct ToolBox(pub std::sync::Arc<dyn Tool>);

/// Error type for tool registry operations.
///
/// Distinguished from [`ToolRegistrarError`] which covers registrar-level
/// errors (conflict reporting, internal failures).
#[derive(Debug, Error)]
pub enum RegistryError {
    /// A tool name was already registered.
    #[error("tool `{0}` already registered")]
    AlreadyRegistered(String),

    /// A tool name was already registered, with full conflict details.
    #[error("tool `{tool}` already registered by `{registrar}`, attempting: `{attempting}`")]
    Conflict {
        /// The conflicting tool name.
        tool: String,
        /// The registrar that registered it first.
        registrar: String,
        /// The registrar that attempted to register the conflicting tool.
        attempting: String,
    },

    /// The registry is frozen — no further registrations accepted.
    #[error("tool registry is frozen — no further registrations accepted")]
    Frozen,

    /// Internal error during registration.
    #[error("{0}")]
    Internal(String),
}

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

// ═══════════════════════════════════════════════════════════════════════════
// ToolRegistrar — module-level tool registration trait
// ═══════════════════════════════════════════════════════════════════════════

/// Error type for tool registration.
#[derive(Debug, Error)]
pub enum ToolRegistrarError {
    /// A tool name was already registered by another registrar.
    #[error("tool `{tool}` already registered by `{registrar}`, attempting: `{attempting}`")]
    Conflict {
        /// The conflicting tool name.
        tool: String,
        /// The registrar that registered it first.
        registrar: String,
        /// The registrar that attempted to register the conflicting tool.
        attempting: String,
    },

    /// Internal error within a registrar.
    #[error("{0}")]
    Internal(String),
}

/// Unified trait for modules that provide tools.
///
/// Each implementation is collected at startup, sorted by
/// [`priority`](Self::priority), and called in order to populate the global
/// [`ToolRegistry`].
#[async_trait]
pub trait ToolRegistrar: Send + Sync {
    /// Unique name for this registrar, used in logs and conflict reports.
    fn name(&self) -> &str;

    /// Priority — lower values are registered first.
    fn priority(&self) -> u32;

    /// Register all tools from this module into `registry`.
    ///
    /// # Errors
    /// Returns [`ToolRegistrarError::Conflict`] if a tool with the same name
    /// already exists in `registry`. Returns [`ToolRegistrarError::Internal`]
    /// for any other registration failure.
    async fn register(&self, registry: &dyn ToolRegistry) -> Result<(), ToolRegistrarError>;
}

// ═══════════════════════════════════════════════════════════════════════════
// ToolRegistry — central registry interface
// ═══════════════════════════════════════════════════════════════════════════

/// Central tool registry interface.
///
/// Provides registration, freezing, indexing, and querying operations.
/// Concrete implementation lives in the tools crate.
#[async_trait]
pub trait ToolRegistry: ToolRegistryQuery + Send + Sync {
    /// Register a type-erased tool.
    ///
    /// The tool is provided as `Box<dyn Any + Send + Sync>` so that common
    /// does not need to depend on the tools crate's `Tool` trait.
    /// Implementations downcast internally.
    ///
    /// `registrar_name` identifies which registrar is registering the tool,
    /// used for conflict error reporting.
    ///
    /// # Errors
    /// Returns `Err` if the registry is frozen or the tool name conflicts.
    async fn register_any(
        &self,
        tool: Box<dyn std::any::Any + Send + Sync>,
        registrar_name: &str,
    ) -> Result<(), RegistryError>;

    /// Mark registration as complete; reject further registrations.
    fn freeze(&self);

    /// Returns whether the registry is frozen.
    fn is_frozen(&self) -> bool;

    /// Build a first-level tools index string, grouped by tool group.
    ///
    /// Eager (non-deferred) tools show name and behavior description.
    /// Deferred tools show name and danger marks only.
    async fn build_index(&self) -> String;
}

// ═══════════════════════════════════════════════════════════════════════════
// ToolRegistryQuery — read-only query interface
// ═══════════════════════════════════════════════════════════════════════════

/// Read-only query interface for the tool registry.
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

// ═══════════════════════════════════════════════════════════════════════════
// Registration helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Register a single tool, converting [`RegistryError`] into [`ToolRegistrarError`].
pub async fn register_tool(
    registry: &dyn ToolRegistry,
    tool: impl Tool + 'static,
    registrar_name: &str,
) -> Result<(), ToolRegistrarError> {
    let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(ToolBox(std::sync::Arc::new(tool)));
    registry
        .register_any(boxed, registrar_name)
        .await
        .map_err(|e| match e {
            RegistryError::Conflict {
                tool,
                registrar,
                attempting,
            } => ToolRegistrarError::Conflict {
                tool,
                registrar,
                attempting,
            },
            RegistryError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
                tool: name,
                registrar: String::new(),
                attempting: registrar_name.to_string(),
            },
            other => ToolRegistrarError::Internal(other.to_string()),
        })
}

/// Register a single tool, logging `Internal` errors as warnings.
///
/// Returns `Ok(true)` on success, `Ok(false)` on recoverable error,
/// or `Err` on conflict.
pub async fn register_single(
    registry: &dyn ToolRegistry,
    name: String,
    tool: impl Tool + 'static,
    registrar_name: &str,
) -> Result<bool, ToolRegistrarError> {
    match register_tool(registry, tool, registrar_name).await {
        Ok(()) => Ok(true),
        Err(ToolRegistrarError::Conflict {
            tool: conflicting,
            registrar,
            attempting,
        }) => Err(ToolRegistrarError::Conflict {
            tool: conflicting,
            registrar,
            attempting,
        }),
        Err(ToolRegistrarError::Internal(e)) => {
            tracing::warn!(
                "{registrar_name}: failed to register \
                 tool `{name}`: {e}"
            );
            Ok(false)
        }
    }
}
