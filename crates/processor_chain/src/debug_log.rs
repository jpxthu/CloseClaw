//! Debug log helpers for the Processor Chain module.
//!
//! Provides structured debug-log emission for processor chain lifecycle events:
//! inbound/outbound processing, routing decisions, and middleware execution.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type ProcessorChainDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type ProcessorChainEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Processor Chain module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"processor_chain"`.
pub fn emit_processor_chain_event(params: ProcessorChainEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_debug_log::LogLevel;

    #[test]
    fn empty_trace_id_is_noop() {
        let ctx = ProcessorChainDebugLogContext::new(None, "", None);
        emit_processor_chain_event(ProcessorChainEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "processor_chain",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = ProcessorChainDebugLogContext::new(None, "trace-123", None);
        emit_processor_chain_event(ProcessorChainEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "processor_chain",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = ProcessorChainDebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }
}
