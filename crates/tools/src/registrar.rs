//! ToolRegistrar — 模块级工具注册 trait
//!
//! 每个工具提供模块实现此 trait，通过统一的 `register_all` 编排完成全局注册。
//! 参见 `docs/design/tools/tool-registrar.md`。

use std::sync::Arc;

use closeclaw_common::tool_registry::ToolRegistrarError;

use crate::registry::{RegistryError, ToolBox};
use crate::Tool;

// Re-export so that `crate::registrar::ToolRegistrar` still resolves.
pub use closeclaw_common::tool_registry::ToolRegistrar;

/// Register a single tool, converting [`RegistryError`] into [`ToolRegistrarError`].
pub(crate) async fn register_tool(
    registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    tool: impl Tool + 'static,
    registrar_name: &str,
) -> Result<(), ToolRegistrarError> {
    let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(ToolBox(Arc::new(tool)));
    registry
        .register_any(boxed, registrar_name)
        .await
        .map_err(|e| {
            // Try to downcast to our concrete RegistryError first.
            if let Some(re) = e.downcast_ref::<RegistryError>() {
                match re {
                    RegistryError::Conflict {
                        tool,
                        registrar,
                        attempting,
                    } => ToolRegistrarError::Conflict {
                        tool: tool.clone(),
                        registrar: registrar.clone(),
                        attempting: attempting.clone(),
                    },
                    RegistryError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
                        tool: name.clone(),
                        registrar: String::new(),
                        attempting: registrar_name.to_string(),
                    },
                    other => ToolRegistrarError::Internal(other.to_string()),
                }
            } else {
                ToolRegistrarError::Internal(e.to_string())
            }
        })
}

/// Register a single tool, logging `Internal` errors as warnings.
///
/// Returns `Ok(true)` on success, `Ok(false)` on recoverable error,
/// or `Err` on conflict.
pub async fn register_single(
    registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
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
