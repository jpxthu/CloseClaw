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
/// Currently handles `reasoning_effort` injection for the OpenAI protocol path.
pub struct DeepSeekPlugin;

impl ModelPlugin for DeepSeekPlugin {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let effort = match request.reasoning_level {
            ReasoningLevel::Low => Some("off"),
            ReasoningLevel::Medium => Some("base"),
            ReasoningLevel::High => Some("high"),
            ReasoningLevel::Max => Some("reasoner"),
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
mod tests {
    use super::*;
    use crate::types::InternalRequest;

    fn make_request(level: ReasoningLevel) -> InternalRequest {
        InternalRequest {
            model: "deepseek-reasoner".into(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: Some(256),
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: level,
            turn_count: None,
        }
    }

    #[test]
    fn test_name() {
        assert_eq!(DeepSeekPlugin.name(), "deepseek");
    }

    #[test]
    fn test_low_maps_to_off() {
        let plugin = DeepSeekPlugin;
        let mut req = make_request(ReasoningLevel::Low);
        plugin.before_request(&mut req);
        assert_eq!(
            req.extra_body.get("reasoning_effort"),
            Some(&Value::String("off".into()))
        );
    }

    #[test]
    fn test_medium_maps_to_base() {
        let plugin = DeepSeekPlugin;
        let mut req = make_request(ReasoningLevel::Medium);
        plugin.before_request(&mut req);
        assert_eq!(
            req.extra_body.get("reasoning_effort"),
            Some(&Value::String("base".into()))
        );
    }

    #[test]
    fn test_high_maps_to_high() {
        let plugin = DeepSeekPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);
        assert_eq!(
            req.extra_body.get("reasoning_effort"),
            Some(&Value::String("high".into()))
        );
    }

    #[test]
    fn test_max_maps_to_reasoner() {
        let plugin = DeepSeekPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        plugin.before_request(&mut req);
        assert_eq!(
            req.extra_body.get("reasoning_effort"),
            Some(&Value::String("reasoner".into()))
        );
    }
}
