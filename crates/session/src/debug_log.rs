//! Debug log helpers for the Session module.
//!
//! Provides structured debug-log emission for session lifecycle events:
//! session creation, lookup, and compaction.
//!
//! Follows the same pattern as
//! [`gateway::debug_log_emitter`](closeclaw_gateway::debug_log_emitter).

use closeclaw_debug_log::{DebugLog, LogEvent, LogLevel, TraceContext};

/// Bundles a [`DebugLog`] reference, trace ID, and session key
/// for debug-log emission.
///
/// Created by callers to pass the guard, trace ID, and session key
/// together, reducing the field count in [`EmitEventParams`].
pub struct SessionDebugLogContext<'a> {
    /// Debug log instance; when `None`, the emit is a no-op.
    pub debug_log: Option<&'a DebugLog>,
    /// Trace ID for correlation. When empty, the emit is a no-op.
    pub trace_id: &'a str,
    /// Optional session key for log correlation.
    pub session_key: Option<&'a str>,
}

impl<'a> SessionDebugLogContext<'a> {
    /// Create a new context from a guard, trace ID, and session key.
    pub fn new(
        debug_log: Option<&'a DebugLog>,
        trace_id: &'a str,
        session_key: Option<&'a str>,
    ) -> Self {
        Self {
            debug_log,
            trace_id,
            session_key,
        }
    }
}

/// Parameters for emitting a session debug log event.
///
/// Aggregates all event fields into a struct to keep
/// [`emit_session_event`] within the project's 6-parameter limit.
pub struct SessionEmitEventParams<'a> {
    /// Bundled debug-log context (instance, trace ID, and session key).
    pub ctx: SessionDebugLogContext<'a>,
    /// Log level for the event.
    pub level: LogLevel,
    /// Source module that produced the event.
    pub source_module: &'a str,
    /// Event type identifier (e.g. `"session.created"`).
    pub event_type: &'a str,
    /// Structured event payload.
    pub payload: serde_json::Value,
    /// Optional parent [`TraceContext`] for child span derivation.
    /// When `Some`, creates a child span; when `None`, creates a
    /// root span from `trace_id`.
    pub parent: Option<&'a TraceContext>,
}

/// Emit a structured debug log event for the Session module.
///
/// When `parent` is `Some`, creates a child [`TraceContext`] derived
/// from the parent span. When `parent` is `None`, creates a root
/// [`TraceContext`] from `trace_id` (backward compatible).
///
/// If `trace_id` is empty or `debug_log` is `None`, the call is a
/// no-op.
pub fn emit_session_event(params: SessionEmitEventParams<'_>) {
    if params.ctx.trace_id.is_empty() {
        return;
    }
    let Some(debug_log) = params.ctx.debug_log else {
        return;
    };
    let ctx = match params.parent {
        Some(p) => p.child(),
        None => TraceContext::new_root(params.ctx.trace_id.to_string()),
    };
    let event = LogEvent::new(
        &ctx,
        params.ctx.session_key.map(|s| s.to_string()),
        params.level,
        params.source_module,
        params.event_type,
        params.payload,
    );
    let debug_log = debug_log.clone();
    tokio::spawn(async move {
        debug_log.log(event).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_trace_id_is_noop() {
        // Should not panic when trace_id is empty.
        let ctx = SessionDebugLogContext::new(None, "", None);
        emit_session_event(SessionEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "session",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        // Should not panic when debug_log is None.
        let ctx = SessionDebugLogContext::new(None, "trace-123", None);
        emit_session_event(SessionEmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "session",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn debug_log_context_new_fields() {
        let ctx = SessionDebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }
}
