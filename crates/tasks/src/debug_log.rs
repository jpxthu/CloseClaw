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

    closeclaw_debug_log::debug_log_core_tests!("tasks");

    #[test]
    fn test_generate_trace_id_is_unique() {
        let id1 = generate_trace_id();
        let id2 = generate_trace_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("tasks-"));
        assert!(id2.starts_with("tasks-"));
    }

    #[test]
    fn test_generate_trace_id_has_expected_format() {
        let id = generate_trace_id();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts[0], "tasks");
        assert_eq!(parts.len(), 3); // tasks-<timestamp>-<8char-uuid>
        assert_eq!(parts[2].len(), 8);
    }
}
