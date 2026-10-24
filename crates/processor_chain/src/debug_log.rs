//! Debug log helpers for the Processor Chain module.
//!
//! Provides structured debug-log emission for processor chain lifecycle events:
//! inbound/outbound processing, routing decisions, and middleware execution.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type ProcessorChainDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type ProcessorChainEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the Processor Chain module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"processor_chain"`.
pub fn emit_processor_chain_event(params: ProcessorChainEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    closeclaw_debug_log::debug_log_wrapper_tests!(
        emit_processor_chain_event,
        ProcessorChainDebugLogContext,
        ProcessorChainEmitEventParams,
        "processor_chain"
    );
}
