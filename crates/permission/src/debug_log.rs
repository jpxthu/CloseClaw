//! Debug log helpers for the Permission module.
//!
//! Provides structured debug-log emission for permission lifecycle events:
//! permission checks and approval flow triggers.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type PermissionDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type PermissionEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Permission module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"permission"`.
pub fn emit_permission_event(params: PermissionEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    closeclaw_debug_log::debug_log_wrapper_tests!(
        emit_permission_event,
        PermissionDebugLogContext,
        PermissionEmitEventParams,
        "permission"
    );
}
