//! Anthropic-specific request plugin.
//!
//! Reasoning level downgrade is now handled at the gateway layer via
//! [`resolve_effective_reasoning_level`], not in the plugin.
//! This plugin is a no-op; it is kept for plugin registration.
//!
//! The shared helper [`resolve_anthropic_effective`] is still used by the
//! gateway fallback heuristic for Anthropic models.

use crate::plugin::ModelPlugin;
use crate::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;

/// Anthropic capabilities table.
/// Only `High` is natively supported; Off maps to `Low` (minimum available).
const ANTHROPIC_SUPPORTED_LEVEL: ReasoningLevel = ReasoningLevel::High;
const ANTHROPIC_MIN_AVAILABLE: ReasoningLevel = ReasoningLevel::Low;

/// Resolve effective reasoning level for Anthropic provider.
///
/// Shared helper used by the gateway fallback heuristic
/// (`resolve_effective_reasoning_level`) to avoid duplicating the
/// capability table. The plugin itself no longer calls this.
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

/// Plugin for Anthropic models.
///
/// Downgrade logic has been moved to the gateway layer. This plugin is
/// intentionally a no-op to preserve plugin registration in the pipeline.
pub struct AnthropicPlugin;

impl ModelPlugin for AnthropicPlugin {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn before_request(&self, _request: &mut InternalRequest) {
        // No-op: reasoning level downgrade is now handled at the gateway layer
        // via resolve_effective_reasoning_level.
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
            system_appends: None,
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

    // ── before_request is a no-op ────────────────────────────────────────

    /// Off should not be rewritten to Low; downgrade is now at gateway layer.
    #[test]
    fn test_off_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Off);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Off);
    }

    /// Low should pass through unchanged.
    #[test]
    fn test_low_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Low);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Low);
    }

    /// Medium should pass through unchanged.
    #[test]
    fn test_medium_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Medium);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Medium);
    }

    /// Max should pass through unchanged.
    #[test]
    fn test_max_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::Max);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::Max);
    }

    /// High should pass through unchanged.
    #[test]
    fn test_high_passthrough() {
        let plugin = AnthropicPlugin;
        let mut req = make_request(ReasoningLevel::High);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);
    }

    // ── no extra body injection ──────────────────────────────────────────

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

    // ── all levels roundtrip unchanged ───────────────────────────────────

    #[test]
    fn test_all_levels_pass_through_unchanged() {
        let plugin = AnthropicPlugin;
        let levels = [
            ReasoningLevel::Off,
            ReasoningLevel::Low,
            ReasoningLevel::Medium,
            ReasoningLevel::High,
            ReasoningLevel::Max,
        ];
        for level in levels {
            let mut req = make_request(level);
            plugin.before_request(&mut req);
            assert_eq!(
                req.reasoning_level, level,
                "{level:?} should not be rewritten by AnthropicPlugin"
            );
        }
    }
}
