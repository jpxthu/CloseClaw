//! Tests for `session_handler_announce` (reasoning-level resolution).
//!
//! Moved out of `session_handler_announce.rs` for the 1000-line file
//! hard limit (CONTRIBUTING.md 硬性限制). Mounted as a `#[path]` child
//! module so `use super::*` and private-item access keep working.

use super::*;
use closeclaw_llm::ProviderModelKnowledge;
use closeclaw_session::persistence::ReasoningLevel;

#[test]
fn test_resolve_effective_level_default_fallback_max_to_high() {
    let kb = ProviderModelKnowledge::new();
    // Unknown model + Max → High (Max may not be supported, downgrade).
    let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::Max, &kb);
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_none_returns_requested() {
    // With an empty knowledge base, model not found → requested.
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("some-model", ReasoningLevel::Medium, &kb);
    assert_eq!(result, ReasoningLevel::Medium);
}

#[test]
fn test_resolve_effective_level_default_fallback_off_to_low() {
    // Unknown model + Off → Low (Off may not be closable).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::Off, &kb);
    assert_eq!(result, ReasoningLevel::Low);
}

#[test]
fn test_resolve_effective_level_default_fallback_low_unchanged() {
    // Unknown model + Low → Low (base level, always safe).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::Low, &kb);
    assert_eq!(result, ReasoningLevel::Low);
}

#[test]
fn test_resolve_effective_level_default_fallback_high_unchanged() {
    // Unknown model + High → High (base level, always safe).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::High, &kb);
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_default_fallback_medium_unchanged() {
    // Unknown model + Medium → Medium (base level, always safe).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("unknown-model", ReasoningLevel::Medium, &kb);
    assert_eq!(result, ReasoningLevel::Medium);
}

#[test]
fn test_resolve_effective_level_toggle_on_returns_requested() {
    // Toggle on=true: returns requested level, except Max → High (downgrade).
    let kb = ProviderModelKnowledge::new();
    assert_eq!(
        resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::Low, &kb),
        ReasoningLevel::Low,
    );
    assert_eq!(
        resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::Medium, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::High, &kb),
        ReasoningLevel::High,
    );
    assert_eq!(
        resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::Max, &kb),
        ReasoningLevel::High,
        "Max should downgrade to High on Toggle",
    );
}

#[test]
fn test_resolve_effective_level_toggle_on_off_returns_off() {
    // Toggle on=true + Off request → Off (plugin injects disabled).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::Off, &kb);
    assert_eq!(result, ReasoningLevel::Off);
}

#[test]
fn test_resolve_effective_level_levels_flash() {
    // deepseek-v4-flash: off=true, base=true, reasoner=false
    // Max→High (no reasoner), High→Medium (base), Medium→Medium,
    // Low→Low, Off→Off.
    let kb = ProviderModelKnowledge::new();
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Max, &kb),
        ReasoningLevel::High,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::High, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Medium, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Low, &kb),
        ReasoningLevel::Low,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-flash", ReasoningLevel::Off, &kb),
        ReasoningLevel::Off,
    );
}

#[test]
fn test_resolve_effective_level_levels_pro() {
    // deepseek-v4-pro: off=false, base=true, reasoner=false
    // Max→High (no reasoner), High→Medium (base), Medium→Medium,
    // Low→Low, Off→Low (no off support).
    let kb = ProviderModelKnowledge::new();
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Max, &kb),
        ReasoningLevel::High,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::High, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Medium, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Low, &kb),
        ReasoningLevel::Low,
    );
    assert_eq!(
        resolve_effective_reasoning_level("deepseek-v4-pro", ReasoningLevel::Off, &kb),
        ReasoningLevel::Low,
    );
}

// ── Anthropic fallback (model name heuristic) ───────────────────────────

#[test]
fn test_resolve_effective_level_anthropic_off_via_model_heuristic() {
    // Model name contains "claude" → Anthropic fallback triggered.
    let kb = ProviderModelKnowledge::new();
    let result =
        resolve_effective_reasoning_level("claude-3-5-sonnet-20241022", ReasoningLevel::Off, &kb);
    assert_eq!(result, ReasoningLevel::Low);
}

#[test]
fn test_resolve_effective_level_anthropic_max_via_model_heuristic() {
    let kb = ProviderModelKnowledge::new();
    let result =
        resolve_effective_reasoning_level("claude-3-5-sonnet-20241022", ReasoningLevel::Max, &kb);
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_anthropic_low_via_model_heuristic() {
    let kb = ProviderModelKnowledge::new();
    let result =
        resolve_effective_reasoning_level("claude-3-5-sonnet-20241022", ReasoningLevel::Low, &kb);
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_anthropic_medium_via_model_heuristic() {
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level(
        "claude-3-5-sonnet-20241022",
        ReasoningLevel::Medium,
        &kb,
    );
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_anthropic_high_via_model_heuristic() {
    let kb = ProviderModelKnowledge::new();
    let result =
        resolve_effective_reasoning_level("claude-3-5-sonnet-20241022", ReasoningLevel::High, &kb);
    assert_eq!(result, ReasoningLevel::High);
}

#[test]
fn test_resolve_effective_level_non_anthropic_default_fallback_off_to_low() {
    // Non-Anthropic model not in KB → default heuristic fallback:
    // Off → Low (unknown model may not support disabling reasoning).
    let kb = ProviderModelKnowledge::new();
    let result = resolve_effective_reasoning_level("gpt-4o", ReasoningLevel::Off, &kb);
    assert_eq!(result, ReasoningLevel::Low);
}

#[test]
fn test_resolve_effective_level_toggle_off_returns_low() {
    // Toggle on=false: reasoning unavailable → always Low.
    // No real model has on=false in the knowledge base, so construct
    // a custom entry via the test-only `with_test_model` helper.
    let params = closeclaw_llm::knowledge::ModelRecommendParams {
        context_window: 128_000,
        max_tokens: 8_192,
        default_temperature: 0.7,
        reasoning: true,
        reasoning_levels: closeclaw_llm::knowledge::ReasoningLevels::Toggle { on: false },
        reasoning_always: false,
        input_types: vec![closeclaw_llm::model_info::InputType::Text],
        recommended_protocol: closeclaw_llm::types::ProtocolId::from("openai"),
    };
    let kb =
        ProviderModelKnowledge::new().with_test_model("test_provider", "toggle-off-model", params);

    // All requested levels should resolve to Low when on=false.
    for level in [
        ReasoningLevel::Off,
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::Max,
    ] {
        let result = resolve_effective_reasoning_level("toggle-off-model", level, &kb);
        assert_eq!(
            result,
            ReasoningLevel::Low,
            "Toggle on=false should return Low for requested {level:?}"
        );
    }
}

#[test]
fn test_resolve_effective_level_anthropic_model_name_detection() {
    // is_anthropic_model helper test.
    assert!(is_anthropic_model("claude-3-5-sonnet-20241022"));
    assert!(is_anthropic_model("Claude-3-Haiku"));
    assert!(is_anthropic_model("anthropic-model"));
    assert!(!is_anthropic_model("gpt-4o"));
    assert!(!is_anthropic_model("MiniMax-M2.7"));
    assert!(!is_anthropic_model("default"));
}
