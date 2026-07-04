//! ToolRegistrar — 模块级工具注册 trait
//!
//! 每个工具提供模块实现此 trait，通过统一的 `register_all` 编排完成全局注册。
//! 参见 `docs/design/tools/tool-registrar.md`。

use async_trait::async_trait;
use thiserror::Error;

use crate::{Tool, ToolError, ToolRegistry};

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

/// Register a single tool, converting [`ToolError`] into [`ToolRegistrarError`].
pub(crate) async fn register_tool(
    registry: &ToolRegistry,
    tool: impl Tool + 'static,
    registrar_name: &str,
) -> Result<(), ToolRegistrarError> {
    registry.register(tool).await.map_err(|e| match e {
        ToolError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
            tool: name,
            registrar: registrar_name.to_string(),
        },
        other => ToolRegistrarError::Internal(other.to_string()),
    })
}

/// Register a single tool, logging `Internal` errors as warnings.
///
/// Returns `Ok(true)` on success, `Ok(false)` on recoverable error,
/// or `Err` on conflict.
pub async fn register_single(
    registry: &ToolRegistry,
    name: String,
    tool: impl Tool + Send + 'static,
    registrar_name: &str,
) -> Result<bool, ToolRegistrarError> {
    match register_tool(registry, tool, registrar_name).await {
        Ok(()) => Ok(true),
        Err(ToolRegistrarError::Conflict { .. }) => Err(ToolRegistrarError::Conflict {
            tool: name,
            registrar: registrar_name.to_string(),
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

/// Register a tool and increment the counter on success.
///
/// Used inside registrar `register()` methods to reduce boilerplate.
/// Extracts the tool name, calls [`register_single`], and increments
/// `$registered` when the tool is accepted.
#[macro_export]
macro_rules! try_register {
    ($registry:expr, $registered:expr, $tool:expr, $registrar_name:expr) => {
        let tool = $tool;
        let name = tool.name().to_string();
        if $crate::registrar::register_single($registry, name, tool, $registrar_name).await? {
            $registered += 1;
        }
    };
}
