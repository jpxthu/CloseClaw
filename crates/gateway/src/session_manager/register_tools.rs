//! Tool registration delegation for [`SessionManager`].

use closeclaw_common::{ToolRegistrarError, ToolRegistry};
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::warn;

use super::SessionManager;

/// Callback type for registering session tools into a [`ToolRegistry`].
///
/// Injected by daemon (composition root) so that
/// [`SessionManager::register_tools`] can delegate to the tools crate
/// without gateway depending on it directly.
pub(crate) type ToolRegisterFn = Arc<
    dyn Fn(&dyn ToolRegistry) -> BoxFuture<'static, Result<(), ToolRegistrarError>> + Send + Sync,
>;

/// Set the tool-register callback.
///
/// Called by daemon (composition root) so that [`SessionManager::register_tools`]
/// can delegate to the tools crate without a direct dependency.
pub(crate) async fn set_tool_register_fn(sm: &SessionManager, func: ToolRegisterFn) {
    *sm.tool_register_fn.write().await = Some(func);
}

/// Register session tools into the given [`ToolRegistry`].
///
/// Delegates to the callback set via [`set_tool_register_fn`].
/// If no callback has been registered, this is a no-op with a warning log.
pub(crate) async fn register_tools(
    sm: &SessionManager,
    registry: &dyn ToolRegistry,
) -> Result<(), ToolRegistrarError> {
    let guard = sm.tool_register_fn.read().await;
    match guard.as_ref() {
        Some(func) => func(registry).await,
        None => {
            warn!("session_manager: register_tools called but no tool_register_fn set, skipping");
            Ok(())
        }
    }
}
