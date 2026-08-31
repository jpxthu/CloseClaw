//! Debug log helpers for the LLM module.
//!
//! Provides structured debug-log emission for LLM call lifecycle events:
//! call start/end, retry, failure, and full request/response payloads.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type LlmDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type LlmEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the LLM module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"llm"`.
pub fn emit_llm_event(params: LlmEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_debug_log::LogLevel;

    #[test]
    fn empty_trace_id_is_noop() {
        let ctx = LlmDebugLogContext::new(None, "", None);
        emit_llm_event(LlmEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "llm",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = LlmDebugLogContext::new(None, "trace-123", None);
        emit_llm_event(LlmEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "llm",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = LlmDebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }
}
