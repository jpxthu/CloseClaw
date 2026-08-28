//! Anthropic-specific request plugin.
//!
//! Handles reasoning level downgrade semantics for Anthropic:
//! - `Off → Low` (Anthropic cannot turn off reasoning; lowest available = Low)
//! - `Low → High`, `Medium → High`, `Max → High` (only High is supported)
//! - `High` passes through unchanged
//!
//! Unlike DeepSeek/GLM plugins, Anthropic does not inject any extra body
//! parameters — reasoning control is handled implicitly by the model layer.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;

/// Anthropic capabilities table.
/// Only `High` is natively supported; Off maps to `Low` (minimum available).
const ANTHROPIC_SUPPORTED_LEVEL: ReasoningLevel = ReasoningLevel::High;
const ANTHROPIC_MIN_AVAILABLE: ReasoningLevel = ReasoningLevel::Low;

/// Resolve effective reasoning level for Anthropic provider.
///
/// Used by both `AnthropicPlugin::before_request` (plugin layer) and
/// `resolve_effective_reasoning_level` (gateway effective-level display)
/// to avoid duplicating the capability table.
///
/// - `Off` → `Low` (minimum available; Anthropic cannot turn off reasoning)
/// - `Low` / `Medium` / `Max` → `High` (only High natively supported)
/// - `High` → `High` (no change)
pub fn resolve_anthropic_effective(requested: ReasoningLevel) -> ReasoningLevel {
    match requested {
        ReasoningLevel::Off => ANTHROPIC_MIN_AVAILABLE,
        ReasoningLevel::Low | ReasoningLevel::Medium | ReasoningLevel::Max => {
            ANTHROPIC_SUPPORTED_LEVEL
        }
        ReasoningLevel::High => ANTHROPIC_SUPPORTED_LEVEL,
    }
}

/// Plugin that applies Anthropic-specific reasoning level downgrade semantics.
///
/// Downgrades are performed before the request reaches the protocol layer,
/// ensuring the protocol layer (e.g., OpenAI-compatible) receives a valid
/// reasoning level without needing provider-specific override logic.
pub struct AnthropicPlugin;

impl ModelPlugin for AnthropicPlugin {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn before_request(&self, request: &mut InternalRequest) {
        match request.reasoning_level {
            // Off → Low: cannot turn off reasoning; use minimum available
            ReasoningLevel::Off => {
                tracing::info!(
                    provider = "anthropic",
                    model = %request.model,
                    from = %ReasoningLevel::Off,
                    to = %ANTHROPIC_MIN_AVAILABLE,
                    "reasoning level downgraded: Anthropic does not support Off; \
                     using minimum available level"
                );
                request.reasoning_level = ANTHROPIC_MIN_AVAILABLE;
            }
            // Low/Medium/Max → High: only High is natively supported
            ReasoningLevel::Low | ReasoningLevel::Medium | ReasoningLevel::Max => {
                tracing::info!(
                    provider = "anthropic",
                    model = %request.model,
                    from = %request.reasoning_level,
                    to = %ANTHROPIC_SUPPORTED_LEVEL,
                    "reasoning level downgraded: Anthropic supports High only"
                );
                request.reasoning_level = ANTHROPIC_SUPPORTED_LEVEL;
            }
            // High: no change needed
            ReasoningLevel::High => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InternalRequest;

    fn make_request(level: ReasoningLevel) -> InternalRequest {
        InternalRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
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
        assert_eq!(AnthropicPlugin.name(), "anthropic");
    }

    // ── Off → Low ─────────────────────────────────────────────────────────

    #[test]
    fn test_off_downgrades_to_low() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Low);
    }

    // ── Non-High → High ───────────────────────────────────────────────────

    #[test]
    fn test_low_downgrades_to_high() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Low);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
    }

    #[test]
    fn test_medium_downgrades_to_high() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Medium);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
    }

    #[test]
    fn test_max_downgrades_to_high() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
    }

    // ── High passthrough ──────────────────────────────────────────────────

    #[test]
    fn test_high_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
    }

    // ── No extra body injection ───────────────────────────────────────────

    #[test]
    fn test_no_extra_body_injected() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        plugin.before_request(&mut req);
        assert!(
            req.extra_body.is_empty(),
            "AnthropicPlugin should not inject extra_body parameters"
        );
    }

    // ── Logging path verification ─────────────────────────────────────────

    /// Off→Low downgrade triggers the logging path (tracing::info!).
    #[test]
    fn test_off_downgrade_triggers_logging_path() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        assert_eq!(req.reasoning_level, ReasoningLevel::Off);
        plugin.before_request(&mut req);
        // The downgrade path ran; reasoning_level is now Low
        assert_eq!(req.reasoning_level, ReasoningLevel::Low);
    }

    /// Low/Medium/Max→High downgrades trigger the logging path.
    #[test]
    fn test_non_high_downgrade_triggers_logging_path() {
        let plugin = AnthropicPlugin;
        for level in [
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::Max,
        ] {
            let mut req = make_request(level);
            plugin.before_request(&mut req);
            assert_eq!(req.reasoning_level, ReasoningLevel::High);
        }
    }

    /// High should NOT trigger the downgrade path.
    #[test]
    fn test_high_no_downgrade() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
        assert!(
            req.extra_body.is_empty(),
            "High passthrough should not touch extra_body"
        );
    }
}
