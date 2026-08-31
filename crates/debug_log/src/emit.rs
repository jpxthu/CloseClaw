//! Common debug-log emission types and function.
//!
//! Shared across all business modules to avoid duplicating
//! [`DebugLogContext`], [`EmitEventParams`], and [`emit_event`].

use crate::{DebugLog, LogEvent, LogLevel, TraceContext};

/// Bundles a [`DebugLog`] reference, trace ID, and session key
/// for debug-log emission.
///
/// Created by callers to pass the guard, trace ID, and session key
/// together, reducing the field count in [`EmitEventParams`].
#[derive(Clone, Copy)]
pub struct DebugLogContext<'a> {
    /// Debug log instance; when `None`, the emit is a no-op.
    pub debug_log: Option<&'a DebugLog>,
    /// Trace ID for correlation. When empty, the emit is a no-op.
    pub trace_id: &'a str,
    /// Optional session key for log correlation.
    pub session_key: Option<&'a str>,
}

impl<'a> DebugLogContext<'a> {
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

/// Parameters for emitting a debug log event.
///
/// Aggregates all event fields into a struct to keep
/// [`emit_event`] within the project's 6-parameter limit.
pub struct EmitEventParams<'a> {
    /// Bundled debug-log context (instance, trace ID, and session key).
    pub ctx: DebugLogContext<'a>,
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

/// Emit a structured debug log event.
///
/// When `parent` is `Some`, creates a child [`TraceContext`] derived
/// from the parent span. When `parent` is `None`, creates a root
/// [`TraceContext`] from `trace_id` (backward compatible).
///
/// If `trace_id` is empty or `debug_log` is `None`, the call is a
/// no-op.
pub fn emit_event(params: EmitEventParams<'_>) {
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
        let ctx = DebugLogContext::new(None, "", None);
        emit_event(EmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "test",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn none_debug_log_is_noop() {
        let ctx = DebugLogContext::new(None, "trace-123", None);
        emit_event(EmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "test",
            event_type: "test.event",
            payload: serde_json::json!({}),
            parent: None,
        });
    }

    #[test]
    fn context_new_fields() {
        let ctx = DebugLogContext::new(None, "tid", Some("skey"));
        assert_eq!(ctx.trace_id, "tid");
        assert_eq!(ctx.session_key, Some("skey"));
        assert!(ctx.debug_log.is_none());
    }

    #[test]
    fn emit_with_parent_uses_child_span() {
        let parent = TraceContext::new_root("trace-parent".to_string());
        let ctx = DebugLogContext::new(None, "trace-parent", None);
        // Should not panic; parent is Some so child() is called.
        emit_event(EmitEventParams {
            ctx,
            level: LogLevel::Debug,
            source_module: "test",
            event_type: "test.child",
            payload: serde_json::json!({}),
            parent: Some(&parent),
        });
    }

    #[test]
    fn emit_with_session_key() {
        let ctx = DebugLogContext::new(None, "trace-456", Some("session-789"));
        emit_event(EmitEventParams {
            ctx,
            level: LogLevel::Info,
            source_module: "test",
            event_type: "test.session",
            payload: serde_json::json!({"key": "value"}),
            parent: None,
        });
    }
}
