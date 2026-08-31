//! Debug log helpers for the Slash module.
//!
//! Provides structured debug-log emission for slash command lifecycle events:
//! command detection, dispatch, and execution.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type SlashDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type SlashEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Slash module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"slash"`.
pub fn emit_slash_event(params: SlashEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_debug_log::LogLevel;

    #[test]
    fn empty_trace_id_is_noop() {
        let ctx = SlashDebugLogContext::new(None, "", None);
        emit_slash_event(SlashEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "slash",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = SlashDebugLogContext::new(None, "trace-123", None);
        emit_slash_event(SlashEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "slash",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = SlashDebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }
}
