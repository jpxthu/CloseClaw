//! GLM-specific request plugin.
//!
//! Injects `thinking.type` into [`InternalRequest::extra_body`] based on the
//! configured [`ReasoningLevel`], allowing the protocol layer to forward
//! it to the GLM API.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;

/// Plugin that enriches GLM requests with provider-specific parameters.
///
/// Currently handles `thinking.type` injection based on the configured
/// [`ReasoningLevel`].
pub struct GlmPlugin;

impl ModelPlugin for GlmPlugin {
    fn name(&self) -> &str {
        "glm"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let thinking_type = match request.reasoning_level {
            ReasoningLevel::Off | ReasoningLevel::Low => "disabled",
            ReasoningLevel::Medium | ReasoningLevel::High | ReasoningLevel::Max => "enabled",
        };

        request
            .extra_body
            .insert("thinking".to_string(), json!({"type": thinking_type}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InternalRequest;

    fn make_request(level: ReasoningLevel) -> InternalRequest {
        InternalRequest {
            model: "glm-model".to_string(),
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
        let plugin = GlmPlugin;
        assert_eq!(plugin.name(), "glm");
    }

    #[test]
    fn test_before_request_off_disabled() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "disabled"}));
    }

    #[test]
    fn test_before_request_low_disabled() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::Low);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "disabled"}));
    }

    #[test]
    fn test_before_request_medium_enabled() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::Medium);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_before_request_high_enabled() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_before_request_max_enabled() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    /// Max should remain Max (no downgrade) and map to enabled.
    #[test]
    fn test_max_not_downgraded() {
        let plugin = GlmPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        assert_eq!(req.reasoning_level, ReasoningLevel::Max);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Max);
        assert_eq!(
            req.extra_body.get("thinking"),
            Some(&json!({"type": "enabled"}))
        );
    }
}
