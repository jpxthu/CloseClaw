//! MiniMax-specific request plugin.
//!
//! Conditionally injects `reasoning_split` into [`InternalRequest::extra_body`]
//! when the request involves multi-turn tool calls, allowing the Anthropic
//! protocol layer to forward it to the MiniMax API.
//!
//! For M3 models, injects `thinking: {type: "enabled"}` to enable thinking
//! block output as required by the MiniMax API.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use serde_json::{json, Value};

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
        if request.model.starts_with("MiniMax-M3") {
            request
                .extra_body
                .insert("thinking".to_string(), json!({"type": "enabled"}));
        }
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod plugin_tests;
