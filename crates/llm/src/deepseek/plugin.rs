//! DeepSeek-specific request plugin.
//!
//! Injects `reasoning_effort` into [`InternalRequest::extra_body`] based on the
//! configured [`ReasoningLevel`], allowing the OpenAI protocol layer to forward
//! it to the DeepSeek API.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::Value;

/// DeepSeek supports Low/Medium/High natively.
/// Max is equivalent to High (`reasoner` has no extra effect);
/// downgrade Max→High for clarity and log the downgrade.
fn downgrade_max_to_high(request: &mut InternalRequest) {
    if request.reasoning_level == ReasoningLevel::Max {
        tracing::info!(
            provider = "deepseek",
            model = %request.model,
            from = "max",
            to = "high",
            "reasoning level downgraded: Max is equivalent to High on DeepSeek"
        );
        request.reasoning_level = ReasoningLevel::High;
    }
}

/// Plugin that enriches DeepSeek requests with provider-specific parameters.
///
/// Currently handles `reasoning_effort` injection for the OpenAI protocol path.
pub struct DeepSeekPlugin;

impl ModelPlugin for DeepSeekPlugin {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        downgrade_max_to_high(request);

        let effort = match request.reasoning_level {
            ReasoningLevel::Low => Some("low"),
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
