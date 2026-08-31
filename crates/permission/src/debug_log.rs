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
    use closeclaw_debug_log::LogLevel;

    #[test]
    fn empty_trace_id_is_noop() {
        let ctx = PermissionDebugLogContext::new(None, "", None);
        emit_permission_event(PermissionEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "permission",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = PermissionDebugLogContext::new(None, "trace-123", None);
        emit_permission_event(PermissionEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "permission",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = PermissionDebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }
}
