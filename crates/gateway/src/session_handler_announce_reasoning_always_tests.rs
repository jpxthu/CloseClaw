//! Tests for `resolve_effective_reasoning_level` with `reasoning_always` flag.
//!
//! Verifies that models marked `reasoning_always: true` (e.g. MiMo) downgrade
//! Off → Low, while models with `reasoning_always: false` (e.g. GLM) pass
//! Off through unchanged.

use closeclaw_llm::knowledge::ModelRecommendParams;
use closeclaw_llm::knowledge::ReasoningLevels;
use closeclaw_llm::model_info::InputType;
use closeclaw_llm::types::ProtocolId;
use closeclaw_session::persistence::ReasoningLevel;

use crate::session_handler_announce::resolve_effective_reasoning_level;

use closeclaw_llm::ProviderModelKnowledge;

fn make_test_params(
    reasoning_levels: ReasoningLevels,
    reasoning_always: bool,
) -> ModelRecommendParams {
    ModelRecommendParams {
        context_window: 128_000,
        max_tokens: 8_192,
        default_temperature: 0.7,
        reasoning: true,
        reasoning_levels,
        reasoning_always,
        input_types: vec![InputType::Text],
        recommended_protocol: ProtocolId::from("openai"),
    }
}

#[test]
fn test_mimo_off_downgrades_to_low() {
    // MiMo: Toggle { on: true } + reasoning_always: true
    // Off should downgrade to Low because MiMo always produces reasoning.
    let params = make_test_params(ReasoningLevels::Toggle { on: true }, true);
    let kb = ProviderModelKnowledge::new().with_test_model("mimo", "mimo-v2.5-pro", params);

    let result = resolve_effective_reasoning_level("mimo-v2.5-pro", ReasoningLevel::Off, &kb);
    assert_eq!(
        result,
        ReasoningLevel::Low,
        "MiMo Off should downgrade to Low (reasoning_always=true)"
    );
}

#[test]
fn test_mimo_non_off_levels_unchanged() {
    // MiMo: Toggle { on: true } + reasoning_always: true
    // Non-Off levels should pass through unchanged.
    let params = make_test_params(ReasoningLevels::Toggle { on: true }, true);
    let kb = ProviderModelKnowledge::new().with_test_model("mimo", "mimo-v2.5-pro", params);

    assert_eq!(
        resolve_effective_reasoning_level("mimo-v2.5-pro", ReasoningLevel::Low, &kb),
        ReasoningLevel::Low,
    );
    assert_eq!(
        resolve_effective_reasoning_level("mimo-v2.5-pro", ReasoningLevel::Medium, &kb),
        ReasoningLevel::Medium,
    );
    assert_eq!(
        resolve_effective_reasoning_level("mimo-v2.5-pro", ReasoningLevel::High, &kb),
        ReasoningLevel::High,
    );
    // Max → High (standard Toggle behavior)
    assert_eq!(
        resolve_effective_reasoning_level("mimo-v2.5-pro", ReasoningLevel::Max, &kb),
        ReasoningLevel::High,
    );
}

#[test]
fn test_glm_off_passes_through() {
    // GLM: Toggle { on: true } + reasoning_always: false (default)
    // Off should pass through as Off (GLM can truly disable reasoning).
    let params = make_test_params(ReasoningLevels::Toggle { on: true }, false);
    let kb = ProviderModelKnowledge::new().with_test_model("glm", "glm-5.1", params);

    let result = resolve_effective_reasoning_level("glm-5.1", ReasoningLevel::Off, &kb);
    assert_eq!(
        result,
        ReasoningLevel::Off,
        "GLM Off should remain Off (reasoning_always=false)"
    );
}

#[test]
fn test_glm_non_off_levels_unchanged() {
    // GLM: Toggle { on: true } + reasoning_always: false
    // Non-Off levels should pass through unchanged.
    let params = make_test_params(ReasoningLevels::Toggle { on: true }, false);
    let kb = ProviderModelKnowledge::new().with_test_model("glm", "glm-5.1", params);

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
    );
}
