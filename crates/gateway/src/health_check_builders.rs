//! Builder functions for health-check pipeline inputs.
//!
//! Converts gateway-layer types ([`StreamResult`]) into session-layer
//! health-check inputs ([`HealthCheckInput`], [`HookContext`]). These
//! live in the gateway crate because they depend on
//! [`closeclaw_llm::types`] which is not a dependency of the session
//! crate.

use crate::outbound::StreamResult;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::run_health::{HealthCheckInput, HookContext, HookToolCallInfo};

/// Build a [`HealthCheckInput`] from a [`StreamResult`].
///
/// Extracts content-block presence flags and tool-use metadata from
/// the LLM response. `retry_count` and `turn_duration_ms` are set to
/// zero because the caller does not currently track them at this
/// level.
pub(crate) fn build_health_check_input(result: &StreamResult) -> HealthCheckInput {
    let has_text = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Text(_)));
    let has_tool_calls = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
    let has_thinking = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));

    HealthCheckInput {
        has_text,
        has_tool_calls,
        has_thinking,
        retry_count: 0,
        turn_duration_ms: 0,
        is_structurally_valid: true,
        structural_anomaly_detail: None,
    }
}

/// Build a [`HookContext`] from a [`StreamResult`].
///
/// Populates `text`, `tool_calls`, and `tool_results` from the
/// response. `recent_tool_calls` is left empty — callers with
/// access to session history can enrich it.
pub(crate) fn build_hook_context(result: &StreamResult) -> HookContext {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();

    for block in &result.content_blocks {
        match block {
            ContentBlock::Text(t) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
            ContentBlock::ToolUse { name, input, .. } => {
                tool_calls.push(HookToolCallInfo {
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            ContentBlock::ToolResult { content, .. } => {
                tool_results.push(content.clone());
            }
            _ => {}
        }
    }

    HookContext {
        text,
        tool_calls,
        tool_results,
        recent_tool_calls: Vec::new(),
    }
}
