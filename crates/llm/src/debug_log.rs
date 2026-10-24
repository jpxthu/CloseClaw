//! Debug log helpers for the LLM module.
//!
//! Provides structured debug-log emission for LLM call lifecycle events:
//! call start/end, retry, failure, and full request/response payloads.
//!
//! Delegates to the common [`closeclaw_debug_log::emit_event`] function.

/// Type alias for the common [`closeclaw_debug_log::DebugLogContext`].
pub type LlmDebugLogContext<'a> = closeclaw_debug_log::DebugLogContext<'a>;

/// Type alias for the common [`closeclaw_debug_log::EmitEventParams`].
pub type LlmEmitEventParams<'a> = closeclaw_debug_log::EmitEventParams<'a>;

/// Emit a structured debug log event for the LLM module.
///
/// Thin wrapper around [`closeclaw_debug_log::emit_event`] that fixes
/// the `source_module` to `"llm"`.
pub fn emit_llm_event(params: LlmEmitEventParams<'_>) {
    closeclaw_debug_log::emit_event(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    closeclaw_debug_log::debug_log_wrapper_tests!(
        emit_llm_event,
        LlmDebugLogContext,
        LlmEmitEventParams,
        "llm"
    );
}
