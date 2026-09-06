//! DeepSeek-specific request plugin.
//!
//! Injects `reasoning_effort` into [`InternalRequest::extra_body`] based on the
//! configured [`ReasoningLevel`], allowing the OpenAI protocol layer to forward
//! it to the DeepSeek API.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::Value;

/// Plugin that enriches DeepSeek requests with provider-specific parameters.
///
/// Handles `reasoning_effort` injection for the OpenAI protocol path.
/// Reasoning level downgrade is handled by the gateway layer; this plugin
/// only maps the effective level to the native parameter format.
pub struct DeepSeekPlugin;

impl ModelPlugin for DeepSeekPlugin {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let effort = match request.reasoning_level {
            ReasoningLevel::Off | ReasoningLevel::Low => Some("low"),
            ReasoningLevel::Medium => Some("base"),
            ReasoningLevel::High | ReasoningLevel::Max => Some("high"),
        };

        if let Some(val) = effort {
            request.extra_body.insert(
                "reasoning_effort".to_string(),
                Value::String(val.to_string()),
            );
        }
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod plugin_tests;
