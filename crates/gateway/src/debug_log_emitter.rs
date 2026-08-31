//! Helper functions for emitting structured debug log events.
//!
//! Used by [`Gateway`](super::Gateway) at key message-chain nodes to
//! produce traceable log events via the debug log framework.

use closeclaw_debug_log::TraceContext;

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type DebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

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

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type EmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event.
///
/// Delegates to the common [`closeclaw_debug_log::emit_event`] function.
pub fn emit_debug_event(params: EmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}
