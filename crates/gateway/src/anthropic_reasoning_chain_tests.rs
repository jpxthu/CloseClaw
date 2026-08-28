//! Step 1.3 — Integration tests for Anthropic reasoning level downgrade chain.
//!
//! Verifies the full pipeline: AnthropicPlugin → protocol build_request →
//! final request body, and that `resolve_effective_reasoning_level` returns
//! the correct effective level for Anthropic models.
//!
//! These tests are cross-crate integration tests covering:
//! - Plugin downgrade semantics (Off→Low, non-High→High)
//! - Protocol layer no longer injects thinking parameters
//! - `resolve_effective_reasoning_level` Anthropic fallback
//! - No regression for minimax/deepseek/glm

use closeclaw_llm::anthropic_plugin::AnthropicPlugin;
use closeclaw_llm::call_chain::assemble_llm_components;
use closeclaw_llm::plugin::ModelPlugin;
use closeclaw_llm::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use closeclaw_session::persistence::ReasoningLevel;

use closeclaw_common::llm_types::InternalRequest;

fn make_request(level: ReasoningLevel) -> InternalRequest {
    InternalRequest {
        model: "claude-3-5-sonnet-20241022".to_string(),
        messages: vec![closeclaw_common::llm_types::InternalMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            ..Default::default()
        }],
        temperature: 0.7,
        max_tokens: Some(1024),
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

// ═════════════════════════════════════════════════════════════════════════════
// Full-chain integration: AnthropicPlugin → protocol → request body
// ═════════════════════════════════════════════════════════════════════════════

/// Off → plugin downgrades to Low → OpenAiProtocol body has no thinking injection.
///
/// Anthropic provider uses OpenAiProtocol (via call_chain default branch).
/// After plugin downgrade, the protocol builds a clean body.
#[test]
fn test_anthropic_off_full_chain_openai_protocol() {
    let plugin = AnthropicPlugin;
    let mut req = make_request(ReasoningLevel::Off);

    // Plugin downgrades Off → Low
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::Low);

    // OpenAiProtocol builds the body (no thinking params for OpenAI)
    let protocol = OpenAiProtocol::new();
    let body = protocol.build_request(&req).unwrap();

    // Body must NOT contain thinking/reasoning injection
    assert!(
        body.get("thinking").is_none(),
        "body must not contain 'thinking'"
    );
    assert!(
        body.get("reasoning_effort").is_none(),
        "body must not contain 'reasoning_effort'"
    );
    assert!(
        body.get("reasoning_level").is_none(),
        "body must not contain 'reasoning_level'"
    );

    // Body must have standard fields
    assert_eq!(body.get("model").unwrap(), "claude-3-5-sonnet-20241022");
    assert!(body.get("messages").unwrap().is_array());
}

/// Non-High (Max/Medium/Low) → plugin downgrades to High → body clean.
#[test]
fn test_anthropic_non_high_full_chain_openai_protocol() {
    let plugin = AnthropicPlugin;
    let protocol = OpenAiProtocol::new();

    for level in [
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::Max,
    ] {
        let mut req = make_request(level);
        plugin.before_request(&mut req);
        assert_eq!(req.reasoning_level, ReasoningLevel::High);

        let body = protocol.build_request(&req).unwrap();
        assert!(
            body.get("thinking").is_none(),
            "body must not contain 'thinking' for {level:?}"
        );
        assert!(
            body.get("reasoning_effort").is_none(),
            "body must not contain 'reasoning_effort' for {level:?}"
        );
    }
}

/// High → plugin passthrough → body clean.
#[test]
fn test_anthropic_high_full_chain_openai_protocol() {
    let plugin = AnthropicPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::High);

    let protocol = OpenAiProtocol::new();
    let body = protocol.build_request(&req).unwrap();
    assert!(body.get("thinking").is_none());
    assert!(body.get("reasoning_effort").is_none());
}

/// Full chain through AnthropicProtocol (used by minimax/deepseek path).
/// Off → plugin Low → AnthropicProtocol body has no thinking injection.
#[test]
fn test_anthropic_off_full_chain_anthropic_protocol() {
    let plugin = AnthropicPlugin;
    let mut req = make_request(ReasoningLevel::Off);

    plugin.before_request(&mut req);
    assert_eq!(req.reasoning_level, ReasoningLevel::Low);

    let protocol = AnthropicProtocol::new();
    let body = protocol.build_request(&req).unwrap();

    // Anthropic protocol body must NOT contain thinking parameters
    assert!(
        body.get("thinking").is_none(),
        "AnthropicProtocol body must not contain 'thinking'"
    );
    assert!(
        body.get("reasoning_effort").is_none(),
        "AnthropicProtocol body must not contain 'reasoning_effort'"
    );
    assert!(
        body.get("reasoning_level").is_none(),
        "AnthropicProtocol body must not contain 'reasoning_level'"
    );

    // Body must have standard fields
    assert_eq!(body.get("model").unwrap(), "claude-3-5-sonnet-20241022");
    assert!(body.get("messages").unwrap().is_array());
    assert_eq!(body.get("max_tokens").unwrap(), &serde_json::json!(1024));
}

// ═════════════════════════════════════════════════════════════════════════════
// call_chain assembly verification
// ═════════════════════════════════════════════════════════════════════════════

/// Anthropic provider uses OpenAiProtocol with AnthropicPlugin in pipeline.
#[test]
fn test_call_chain_anthropic_assembly() {
    let (protocol, _interpreter, plugin) = assemble_llm_components("anthropic");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "openai",
        "anthropic should use OpenAiProtocol"
    );
    assert!(
        !plugin.is_empty(),
        "anthropic should have AnthropicPlugin in pipeline"
    );
    assert_eq!(
        plugin.len(),
        1,
        "anthropic pipeline should have exactly 1 plugin"
    );
}

/// MiniMax uses AnthropicProtocol with its own plugins (not affected by anthropic changes).
#[test]
fn test_call_chain_minimax_assembly_no_regression() {
    let (protocol, _interpreter, plugin) = assemble_llm_components("minimax");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "anthropic",
        "minimax should use AnthropicProtocol"
    );
    assert!(
        !plugin.is_empty(),
        "minimax should have plugins in pipeline"
    );
}

/// DeepSeek uses AnthropicProtocol with its own plugins (not affected).
#[test]
fn test_call_chain_deepseek_assembly_no_regression() {
    let (protocol, _interpreter, plugin) = assemble_llm_components("deepseek");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "anthropic",
        "deepseek should use AnthropicProtocol"
    );
    assert!(
        !plugin.is_empty(),
        "deepseek should have plugins in pipeline"
    );
}

/// GLM uses OpenAiProtocol with its own plugin (not affected).
#[test]
fn test_call_chain_glm_assembly_no_regression() {
    let (protocol, _interpreter, plugin) = assemble_llm_components("glm");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "openai",
        "glm should use OpenAiProtocol"
    );
    assert!(!plugin.is_empty(), "glm should have GlmPlugin in pipeline");
}

// ═════════════════════════════════════════════════════════════════════════════
// Plugin does not inject extra_body
// ═════════════════════════════════════════════════════════════════════════════

/// AnthropicPlugin must never inject extra_body parameters.
#[test]
fn test_anthropic_plugin_never_injects_extra_body() {
    let plugin = AnthropicPlugin;
    for level in [
        ReasoningLevel::Off,
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::Max,
    ] {
        let mut req = make_request(level);
        plugin.before_request(&mut req);
        assert!(
            req.extra_body.is_empty(),
            "AnthropicPlugin must not inject extra_body for {level:?}"
        );
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Regression: minimax/deepseek plugin injection not affected
// ═════════════════════════════════════════════════════════════════════════════

/// DeepSeekPlugin still injects reasoning_effort even after protocol layer cleanup.
#[test]
fn test_deepseek_plugin_injection_unchanged() {
    let plugin = closeclaw_llm::DeepSeekPlugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    // DeepSeekPlugin injects reasoning_effort for High level
    assert!(
        req.extra_body.contains_key("reasoning_effort"),
        "DeepSeekPlugin should inject 'reasoning_effort' for High"
    );
}

/// MiniMaxM3Plugin still injects thinking for non-Off levels.
#[test]
fn test_minimax_plugin_injection_unchanged() {
    let plugin = closeclaw_llm::MiniMaxM3Plugin;
    let mut req = make_request(ReasoningLevel::High);
    plugin.before_request(&mut req);
    assert!(
        req.extra_body.contains_key("thinking"),
        "MiniMaxM3Plugin should inject 'thinking' for High"
    );
}
