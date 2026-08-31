//! Helper functions for emitting structured debug log events.
//!
//! Used by [`Gateway`](super::Gateway) at key message-chain nodes to
//! produce traceable log events via the debug log framework.

use closeclaw_debug_log::{DebugLog, LogEvent, LogLevel, TraceContext};

/// Bundles a [`DebugLog`] reference, trace ID, and session key
/// for debug-log emission.
///
/// Created by callers to pass the guard, trace ID, and session key
/// together, reducing the field count in [`EmitEventParams`].
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

/// Reconstruct a root [`TraceContext`] from metadata fields.
///
/// Returns `None` when either `trace_id` or `span_id` is missing from
/// the metadata map.
pub fn root_context_from_metadata(
    metadata: &std::collections::HashMap<String, String>,
) -> Option<TraceContext> {
    let trace_id = metadata.get("trace_id")?;
    let span_id = metadata.get("span_id")?;
    Some(TraceContext {
        trace_id: trace_id.clone(),
        span_id: span_id.clone(),
        parent_span_id: String::new(),
    })
}

/// Reconstruct a root [`TraceContext`] from a [`MessageMetadata`].
///
/// Returns `None` when either `trace_id` or `span_id` is missing.
pub fn root_context_from_message_metadata(
    metadata: &super::session_handler::MessageMetadata,
) -> Option<TraceContext> {
    let trace_id = metadata.trace_id.as_ref()?;
    let span_id = metadata.span_id.as_ref()?;
    Some(TraceContext {
        trace_id: trace_id.clone(),
        span_id: span_id.clone(),
        parent_span_id: String::new(),
    })
}

/// Parameters for emitting a debug log event.
///
/// Aggregates all event fields into a struct to keep
/// [`emit_debug_event`] within the project's 6-parameter limit.
pub struct EmitEventParams<'a> {
    /// Bundled debug-log context (instance, trace ID, and session key).
    pub ctx: DebugLogContext<'a>,
    /// Log level for the event.
    pub level: LogLevel,
    /// Source module that produced the event.
    pub source_module: &'a str,
    /// Event type identifier (e.g. `"message.arrived"`).
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
pub fn emit_debug_event(params: EmitEventParams<'_>) {
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
