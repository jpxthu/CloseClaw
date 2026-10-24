//! Debug log helpers for the Tools module.
//!
//! Provides structured debug-log emission for tool execution lifecycle events:
//! tool execution start/end, full parameters/return values, and permission
//! check results.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type ToolsDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type ToolsEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Tools module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"tools"`.
pub fn emit_tool_event(params: ToolsEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    closeclaw_debug_log::debug_log_wrapper_tests!(
        emit_tool_event,
        ToolsDebugLogContext,
        ToolsEmitEventParams,
        "tools"
    );
}
