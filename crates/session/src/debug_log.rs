//! Debug log helpers for the Session module.
//!
//! Provides structured debug-log emission for session lifecycle events:
//! session creation, lookup, and compaction.
//!
//! Delegates to the common [`emit_event`](closeclaw_debug_log::emit_event) function.

/// Type alias for the common [`DebugLogContext`](closeclaw_debug_log::DebugLogContext).
pub type SessionDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`EmitEventParams`](closeclaw_debug_log::EmitEventParams).
pub type SessionEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Session module.
///
/// Thin wrapper around [`emit_event`](closeclaw_debug_log::emit_event) that fixes
/// the `source_module` to `"session"`.
pub fn emit_session_event(params: SessionEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    closeclaw_debug_log::debug_log_wrapper_tests!(
        emit_session_event,
        SessionDebugLogContext,
        SessionEmitEventParams,
        "session"
    );
}
