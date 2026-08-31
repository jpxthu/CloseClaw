//! Helper functions for emitting structured debug log events.
//!
//! Used by [`Gateway`](super::Gateway) at key message-chain nodes to
//! produce traceable log events via the debug log framework.

use closeclaw_debug_log::{DebugLog, LogEvent, LogLevel, TraceContext};

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

/// Emit a structured debug log event if the [`DebugLog`] is configured.
///
/// When `parent` is `Some`, creates a child [`TraceContext`] derived
/// from the parent span. When `parent` is `None`, creates a root
/// [`TraceContext`] from `trace_id` (backward compatible).
///
/// If `trace_id` is empty or `debug_log` is `None`, the call is a
/// no-op.
#[allow(clippy::too_many_arguments)]
pub fn emit_debug_event(
    debug_log: Option<&DebugLog>,
    trace_id: &str,
    session_key: Option<&str>,
    level: LogLevel,
    source_module: &str,
    event_type: &str,
    payload: serde_json::Value,
    parent: Option<&TraceContext>,
) {
    if trace_id.is_empty() {
        return;
    }
    let Some(debug_log) = debug_log else {
        return;
    };
    let ctx = match parent {
        Some(p) => p.child(),
        None => TraceContext::new_root(trace_id.to_string()),
    };
    let event = LogEvent::new(
        &ctx,
        session_key.map(|s| s.to_string()),
        level,
        source_module,
        event_type,
        payload,
    );
    let debug_log = debug_log.clone();
    tokio::spawn(async move {
        debug_log.log(event).await;
    });
}

/// Emit a child debug log event derived from a parent [`TraceContext`].
///
/// Calls `parent.child()` to create a child span, builds a
/// [`LogEvent`], and spawns an async task to write it. If `debug_log`
/// is `None`, the call is a no-op.
#[allow(dead_code)]
pub fn emit_debug_child_event(
    debug_log: Option<&DebugLog>,
    parent: &TraceContext,
    session_key: Option<&str>,
    level: LogLevel,
    source_module: &str,
    event_type: &str,
    payload: serde_json::Value,
) {
    let Some(debug_log) = debug_log else {
        return;
    };
    let ctx = parent.child();
    let event = LogEvent::new(
        &ctx,
        session_key.map(|s| s.to_string()),
        level,
        source_module,
        event_type,
        payload,
    );
    let debug_log = debug_log.clone();
    tokio::spawn(async move {
        debug_log.log(event).await;
    });
}
