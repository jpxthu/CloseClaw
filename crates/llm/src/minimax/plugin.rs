//! MiniMax-specific request plugin.
//!
//! Conditionally injects `reasoning_split` into [`InternalRequest::extra_body`]
//! when the request involves multi-turn tool calls, allowing the Anthropic
//! protocol layer to forward it to the MiniMax API.
//!
//! For M3 models, injects `thinking: {type: "enabled"}` when reasoning level
//! is High/Max, or `thinking: {type: "disabled"}` when reasoning level is
//! Low/Medium, as required by the MiniMax API.

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

/// Plugin that enriches MiniMax requests with provider-specific parameters.
///
/// Handles conditional `reasoning_split` injection: the flag is set only when
/// the request carries tool definitions **and** the message history already
/// contains tool-result messages (i.e. a multi-turn tool-call scenario).
/// Outside that scenario the parameter is omitted to avoid unnecessary
/// overhead.
pub struct MiniMaxPlugin;

impl ModelPlugin for MiniMaxPlugin {
    fn name(&self) -> &str {
        "minimax"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let has_tool_definitions = request.tools.is_some();
        let has_tool_result_messages = request.messages.iter().any(|m| m.tool_call_id.is_some());

        if has_tool_definitions && has_tool_result_messages {
            request
                .extra_body
                .insert("reasoning_split".to_string(), Value::Bool(true));
        }

        // M3 requires explicit `thinking` parameter to produce thinking blocks.
        // High/Max → enabled, Low/Medium → disabled (binary toggle per design doc).
        // Max is equivalent to High; downgrade before matching.
        if request.model.starts_with("MiniMax-M3") {
            downgrade_max_to_high_m3(request);
            let thinking_type = match request.reasoning_level {
                ReasoningLevel::High => "enabled",
                ReasoningLevel::Low | ReasoningLevel::Medium => "disabled",
                ReasoningLevel::Max => unreachable!("Max should have been downgraded to High"),
            };
            request
                .extra_body
                .insert("thinking".to_string(), json!({"type": thinking_type}));
        }
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod plugin_tests;
