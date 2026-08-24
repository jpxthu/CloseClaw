//! MiniMax-specific request plugins.
//!
//! MiniMax models are split into two per-model plugins:
//! - [`MiniMaxM3Plugin`] — applies to `MiniMax-M3*` models; injects
//!   `thinking: {type: "enabled/disabled"}` based on reasoning level and
//!   handles Max→High downgrade.
//! - [`MiniMaxM2Plugin`] — applies to all other MiniMax models (e.g. M2.7);
//!   conditionally injects `reasoning_split` for multi-turn tool-call
//!   scenarios.
//!
//! The [`ModelPlugin::applies_to`] mechanism on [`PluginPipeline`] ensures
//! each plugin only runs for its target models.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::{json, Value};

/// MiniMax M3 supports High/Max (enabled) and Low/Medium (disabled).
/// Max is equivalent to High; downgrade Max→High and log the downgrade.
fn downgrade_max_to_high_m3(request: &mut InternalRequest) {
    if request.reasoning_level == ReasoningLevel::Max {
        tracing::info!(
            provider = "minimax",
            model = %request.model,
            from = "max",
            to = "high",
            "reasoning level downgraded: Max is equivalent to High on MiniMax M3"
        );
        request.reasoning_level = ReasoningLevel::High;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MiniMaxM3Plugin
// ─────────────────────────────────────────────────────────────────────────────

/// Plugin for MiniMax M3-family models (`MiniMax-M3*`).
///
/// Injects `thinking: {type: "enabled"}` when reasoning level is High/Max
/// (Max is downgraded to High first), or `thinking: {type: "disabled"}` when
/// Low/Medium, as required by the MiniMax M3 API.
///
/// Also injects `reasoning_split` for multi-turn tool-call scenarios (same as
/// M2 — M3 also benefits from this flag when tool calls are present).
pub struct MiniMaxM3Plugin;

impl ModelPlugin for MiniMaxM3Plugin {
    fn name(&self) -> &str {
        "minimax-m3"
    }

    fn applies_to(&self, model: &str) -> bool {
        model.to_lowercase().starts_with("minimax-m3")
    }

    fn before_request(&self, request: &mut InternalRequest) {
        // M3 requires explicit `thinking` parameter to produce thinking blocks.
        // High/Max → enabled, Low/Medium → disabled (binary toggle per design doc).
        // Max is equivalent to High; downgrade before matching.
        downgrade_max_to_high_m3(request);
        let thinking_type = match request.reasoning_level {
            ReasoningLevel::High => "enabled",
            ReasoningLevel::Off | ReasoningLevel::Low | ReasoningLevel::Medium => "disabled",
            ReasoningLevel::Max => unreachable!("Max should have been downgraded to High"),
        };
        request
            .extra_body
            .insert("thinking".to_string(), json!({"type": thinking_type}));

        // Also inject reasoning_split for multi-turn tool calls.
        let has_tool_definitions = request.tools.is_some();
        let has_tool_result_messages = request.messages.iter().any(|m| m.tool_call_id.is_some());
        if has_tool_definitions && has_tool_result_messages {
            request
                .extra_body
                .insert("reasoning_split".to_string(), Value::Bool(true));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MiniMaxM2Plugin
// ─────────────────────────────────────────────────────────────────────────────

/// Plugin for non-M3 MiniMax models (e.g. M2.7, legacy models).
///
/// Conditionally injects `reasoning_split` into [`InternalRequest::extra_body`]
/// when the request involves multi-turn tool calls, allowing the Anthropic
/// protocol layer to forward it to the MiniMax API.
pub struct MiniMaxM2Plugin;

impl ModelPlugin for MiniMaxM2Plugin {
    fn name(&self) -> &str {
        "minimax-m2"
    }

    fn applies_to(&self, model: &str) -> bool {
        // Matches MiniMax models that are NOT M3-family.
        // Case-insensitive: "minimax-*" and "MiniMax-*" both match.
        model.to_lowercase().starts_with("minimax")
            && !model.to_lowercase().starts_with("minimax-m3")
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let has_tool_definitions = request.tools.is_some();
        let has_tool_result_messages = request.messages.iter().any(|m| m.tool_call_id.is_some());

        if has_tool_definitions && has_tool_result_messages {
            request
                .extra_body
                .insert("reasoning_split".to_string(), Value::Bool(true));
        }
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod plugin_tests;
