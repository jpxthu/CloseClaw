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
/// the LLM response. `retry_count` and `turn_duration_ms` are
/// provided by the caller which tracks them at the turn level.
pub(crate) fn build_health_check_input(
    result: &StreamResult,
    turn_duration_ms: u64,
) -> HealthCheckInput {
    let mut is_structurally_valid = true;
    let mut structural_anomaly_detail: Option<String> = None;

    for block in &result.content_blocks {
        if is_structurally_valid {
            match block {
                ContentBlock::ToolUse { id, name, .. } => {
                    if id.is_empty() {
                        is_structurally_valid = false;
                        structural_anomaly_detail = Some("ToolUse block has empty id".into());
                    } else if name.is_empty() {
                        is_structurally_valid = false;
                        structural_anomaly_detail = Some("ToolUse block has empty name".into());
                    }
                }
                ContentBlock::ToolResult { tool_call_id, .. } if tool_call_id.is_empty() => {
                    is_structurally_valid = false;
                    structural_anomaly_detail =
                        Some("ToolResult block has empty tool_call_id".into());
                }
                _ => {}
            }
        }
    }

    let has_tool_calls = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));

    let has_tool_results = result
        .content_blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::ToolResult { .. }));

    HealthCheckInput {
        has_text: result
            .content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Text(_))),
        has_tool_calls,
        has_thinking: result
            .content_blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Thinking { .. })),
        retry_count: result.retry_attempts,
        turn_duration_ms,
        is_structurally_valid,
        structural_anomaly_detail,
        side_effect_occurred: has_tool_results,
    }
}

/// Build a [`HookContext`] from a [`StreamResult`] and recent
/// tool-call history.
///
/// Populates `text`, `tool_calls`, and `tool_results` from the
/// response.  `recent_tool_calls` carries tool-call data from
/// previous turns so that loop-check and progress-check hooks
/// can detect repetitive patterns.
pub(crate) fn build_hook_context(
    result: &StreamResult,
    recent_tool_calls: Vec<HookToolCallInfo>,
) -> HookContext {
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
        recent_tool_calls,
    }
}
