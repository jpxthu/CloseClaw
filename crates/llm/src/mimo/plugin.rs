//! MiMo-specific request plugin.
//!
//! Injects `thinking.type` into [`InternalRequest::extra_body`] based on the
//! configured [`ReasoningLevel`], allowing the protocol layer to forward
//! it to the MiMo API.
//!
//! MiMo uses a binary toggle for thinking:
//! - `off` → `"disabled"`
//! - `low` / `medium` / `high` / `max` → `"enabled"` (max maps directly,
//!   no downgrade needed)

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;

/// Plugin that enriches MiMo requests with provider-specific parameters.
///
/// Currently handles `thinking.type` injection based on the configured
/// [`ReasoningLevel`].
pub struct MimoPlugin;

impl ModelPlugin for MimoPlugin {
    fn name(&self) -> &str {
        "mimo"
    }

    fn applies_to(&self, _model: &str) -> bool {
        // Pipeline is only attached for the mimo provider, so all models
        // within this pipeline are mimo models — no model-level filtering.
        true
    }

    fn before_request(&self, request: &mut InternalRequest) {
        let thinking_type = match request.reasoning_level {
            ReasoningLevel::Off => "disabled",
            ReasoningLevel::Low
            | ReasoningLevel::Medium
            | ReasoningLevel::High
            | ReasoningLevel::Max => "enabled",
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
            model: "mimo-v2.5".to_string(),
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
        let plugin = MimoPlugin;
        assert_eq!(plugin.name(), "mimo");
    }

    #[test]
    fn test_applies_to_any_model() {
        let plugin = MimoPlugin;
        assert!(plugin.applies_to("mimo-v2.5"));
        assert!(plugin.applies_to("mimo-v2-flash"));
        assert!(plugin.applies_to("any-model"));
    }

    // ── Binary mapping: off → disabled, all others → enabled ─────────────

    #[test]
    fn test_off_maps_to_disabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "disabled"}));
    }

    #[test]
    fn test_low_maps_to_enabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::Low);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_medium_maps_to_enabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::Medium);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_high_maps_to_enabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_max_maps_to_enabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    // ── Extra body structure ─────────────────────────────────────────────

    #[test]
    fn test_injects_thinking_key_with_type_field() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert!(thinking.is_object(), "thinking should be an object");
        assert!(
            thinking.get("type").is_some(),
            "thinking should have a 'type' field"
        );
    }

    // ── Model coverage: flash and main models behave identically ──────────

    #[test]
    fn test_flash_model_gets_thinking_injection() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::High);
        req.model = "mimo-v2-flash".to_string();
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "enabled"}));
    }

    #[test]
    fn test_flash_model_off_disabled() {
        let plugin = MimoPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        req.model = "mimo-v2-flash".to_string();
        plugin.before_request(&mut req);

        let thinking = req.extra_body.get("thinking").unwrap();
        assert_eq!(thinking, &json!({"type": "disabled"}));
    }
}
