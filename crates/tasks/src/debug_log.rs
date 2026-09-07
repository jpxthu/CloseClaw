//! Debug log helpers for the Tasks module.
//!
//! Provides a trace-ID generator for background task lifecycle events.
//! Actual event emission uses [`closeclaw_debug_log::emit_event`] directly
//! — no intermediate wrappers.

/// Generate a self-contained trace ID for a non-message internal event.
///
/// Per `docs/design/debug_log/README.md`, non-message events (like background
/// tasks) produce their own trace ID from a system timestamp and module
/// identifier. Best-effort: if `SystemTime` is before UNIX_EPOCH (clock
/// skew), the duration defaults to 0.
pub fn generate_trace_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("tasks-{}-{}", ts, &uuid::Uuid::new_v4().to_string()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_debug_log_is_noop() {
        use closeclaw_debug_log::{emit_event, DebugLogContext, EmitEventParams, LogLevel};
        let ctx = DebugLogContext::new(None, "trace-123", None);
        emit_event(EmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "tasks",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        use closeclaw_debug_log::DebugLogContext;
        let ctx = DebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }

    #[test]
    fn generate_trace_id_is_unique() {
        let id1 = generate_trace_id();
        let id2 = generate_trace_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("tasks-"));
        assert!(id2.starts_with("tasks-"));
    }

    #[test]
    fn generate_trace_id_has_expected_format() {
        let id = generate_trace_id();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts[0], "tasks");
        assert_eq!(parts.len(), 3); // tasks-<timestamp>-<8char-uuid>
        assert_eq!(parts[2].len(), 8);
    }
}
