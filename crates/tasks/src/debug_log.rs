//! Debug log helpers for the Tasks module.
//!
//! Provides structured debug-log emission for background task lifecycle events:
//! task start and terminal state transitions (completed / failed / killed).
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type TasksDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type TasksEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Tasks module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"tasks"`.
pub fn emit_task_event(params: TasksEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

/// Generate a self-contained trace ID for a non-message internal event.
///
/// Per `docs/design/debug_log/README.md`, non-message events (like background
/// tasks) produce their own trace ID from a system timestamp and module
/// identifier.
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
    fn empty_trace_id_is_noop() {
        let ctx = TasksDebugLogContext::new(None, "", None);
        emit_task_event(TasksEmitEventParams {
            ctx,
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tasks",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = TasksDebugLogContext::new(None, "trace-123", None);
        emit_task_event(TasksEmitEventParams {
            ctx,
            level: closeclaw_debug_log::LogLevel::Info,
            source_module: "tasks",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = TasksDebugLogContext::new(None, "tid", Some("skey"));
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
