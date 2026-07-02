//! ToolRegistrar — 模块级工具注册 trait
//!
//! 每个工具提供模块实现此 trait，通过统一的 `register_all` 编排完成全局注册。
//! 参见 `docs/design/tools/tool-registrar.md`。

use async_trait::async_trait;
use thiserror::Error;

use crate::ToolRegistry;

/// Error type for tool registration.
#[derive(Debug, Error)]
pub enum ToolRegistrarError {
    /// A tool name was already registered by another registrar.
    #[error("tool `{tool}` already registered by `{registrar}`")]
    Conflict {
        /// The conflicting tool name.
        tool: String,
        /// The registrar that registered it first.
        registrar: String,
    },

    /// Internal error within a registrar.
    #[error("{0}")]
    Internal(String),
}

/// Unified trait for modules that provide tools.
///
/// Each implementation is collected at startup, sorted by [`priority`](Self::priority),
/// and called in order to populate the global [`ToolRegistry`].
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
    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError>;
}
