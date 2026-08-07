//! Helper functions for emitting structured debug log events.
//!
//! Used by [`Gateway`](super::Gateway) at key message-chain nodes to
//! produce traceable log events via the debug log framework.

use closeclaw_debug_log::{DebugLog, LogEvent, LogLevel, TraceContext};

/// Emit a structured debug log event if the [`DebugLog`] is configured.
///
/// Creates a root [`TraceContext`] from `trace_id`, builds a
/// [`LogEvent`], and spawns an async task to write it. If `trace_id`
/// is empty or `debug_log` is `None`, the call is a no-op.
pub fn emit_debug_event(
    debug_log: Option<&DebugLog>,
    trace_id: &str,
    session_key: Option<&str>,
    level: LogLevel,
    source_module: &str,
    event_type: &str,
    payload: serde_json::Value,
) {
    if trace_id.is_empty() {
        return;
    }
    let Some(debug_log) = debug_log else {
        return;
    };
    let ctx = TraceContext::new_root(trace_id.to_string());
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
