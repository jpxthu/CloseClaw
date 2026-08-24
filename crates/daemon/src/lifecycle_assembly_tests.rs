//! Assembly correctness tests for the LLM call chain.
//!
//! Verifies that each provider_id is assembled with the correct
//! ChatProtocol, Interpreter, and Plugin combination per the design doc
//! (`docs/design/llm/README.md`).
//!
//! The tests mirror the match block in `lifecycle.rs` — they call the same
//! constructor paths (`AnthropicProtocol::new()`, `OpenAiProtocol::new()`,
//! `InterpreterRegistry::new(vec![(…, "provider/*")])`,
//! `PluginPipeline::new().add(…)`) so any sign-change in a constructor
//! will surface here as a compile or test failure.

use closeclaw_llm::interpreter::InterpreterRegistry;
use closeclaw_llm::plugin::PluginPipeline;
use closeclaw_llm::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use closeclaw_llm::types::InternalRequest;
use closeclaw_session::persistence::ReasoningLevel;

// ── Test helpers ─────────────────────────────────────────────────────────────

/// Replicate the per-provider assembly from `lifecycle.rs` for test isolation.
/// Returns `(protocol, interpreter_registry, plugin_pipeline)`.
fn assemble_for_provider(
    provider_id: &str,
) -> (Box<dyn ChatProtocol>, InterpreterRegistry, PluginPipeline) {
    match provider_id {
        "minimax" => (
            Box::new(AnthropicProtocol::new()),
            InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::MinimaxInterpreter),
                "minimax/*",
            )]),
            PluginPipeline::new().add(Box::new(closeclaw_llm::MiniMaxPlugin)),
        ),
        "deepseek" => (
            Box::new(AnthropicProtocol::new()),
            InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::DeepSeekInterpreter),
                "deepseek/*",
            )]),
            PluginPipeline::new().add(Box::new(closeclaw_llm::DeepSeekPlugin)),
        ),
        "glm" => (
            Box::new(OpenAiProtocol::new()),
            InterpreterRegistry::new(vec![(Box::new(closeclaw_llm::GlmInterpreter), "glm/*")]),
            PluginPipeline::new().add(Box::new(closeclaw_llm::GlmPlugin)),
        ),
        _ => (
            Box::new(OpenAiProtocol::new()),
            InterpreterRegistry::default(),
            PluginPipeline::new(),
        ),
    }
}

/// Build a minimal `InternalRequest` for plugin testing.
fn make_request() -> InternalRequest {
    InternalRequest {
        model: "test-model".to_string(),
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
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

// ── 1. Minimax: AnthropicProtocol + MinimaxInterpreter + MiniMaxPlugin ──────

#[test]
fn test_minimax_uses_anthropic_protocol() {
    let (protocol, _, _) = assemble_for_provider("minimax");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "anthropic",
        "minimax should use Anthropic protocol"
    );
}

#[test]
fn test_minimax_interpreter_resolves_for_all_models() {
    let (_, registry, _) = assemble_for_provider("minimax");
    // Any minimax model should resolve to MinimaxInterpreter
    let interp = registry.resolve("minimax", "MiniMax-M3");
    assert_eq!(
        interp.name(),
        "minimax",
        "minimax/* glob should match MiniMax-M3"
    );
    let interp2 = registry.resolve("minimax", "MiniMax-M1");
    assert_eq!(
        interp2.name(),
        "minimax",
        "minimax/* glob should match MiniMax-M1"
    );
}

#[test]
fn test_minimax_plugin_injects_reasoning_split_on_multiturn_tool() {
    let (_, _, pipeline) = assemble_for_provider("minimax");
    assert_eq!(pipeline.len(), 1, "minimax pipeline should have 1 plugin");

    // Multi-turn tool call: tools + messages with tool_call_id
    let mut req = make_request();
    req.tools = Some(vec![]);
    req.messages.push(closeclaw_common::InternalMessage {
        role: "tool".to_string(),
        content: "result".to_string(),
        tool_call_id: Some("tc_1".to_string()),
    });
    pipeline.before_request(&mut req);

    let reasoning_split = req.extra_body.get("reasoning_split");
    assert!(
        reasoning_split.is_some(),
        "MiniMaxPlugin should inject reasoning_split for multi-turn tool calls"
    );
    assert_eq!(reasoning_split.unwrap(), &serde_json::Value::Bool(true));
}

#[test]
fn test_minimax_plugin_no_reasoning_split_without_tool_result() {
    let (_, _, pipeline) = assemble_for_provider("minimax");

    // Tools present but no tool result messages — should NOT inject
    let mut req = make_request();
    req.tools = Some(vec![]);
    pipeline.before_request(&mut req);

    assert!(
        req.extra_body.get("reasoning_split").is_none(),
        "MiniMaxPlugin should NOT inject reasoning_split without tool results"
    );
}

// ── 2. DeepSeek: AnthropicProtocol + DeepSeekInterpreter + DeepSeekPlugin ──

#[test]
fn test_deepseek_uses_anthropic_protocol() {
    let (protocol, _, _) = assemble_for_provider("deepseek");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "anthropic",
        "deepseek should use Anthropic protocol"
    );
}

#[test]
fn test_deepseek_interpreter_resolves_for_all_models() {
    let (_, registry, _) = assemble_for_provider("deepseek");
    let interp = registry.resolve("deepseek", "deepseek-reasoner");
    assert_eq!(
        interp.name(),
        "deepseek",
        "deepseek/* glob should match deepseek-reasoner"
    );
    let interp2 = registry.resolve("deepseek", "deepseek-chat");
    assert_eq!(
        interp2.name(),
        "deepseek",
        "deepseek/* glob should match deepseek-chat"
    );
}

#[test]
fn test_deepseek_interpreter_signature_forwarding() {
    use closeclaw_llm::types::{InternalResponse, RawContentBlock, RawUsage};

    let (_, registry, _) = assemble_for_provider("deepseek");
    let interp = registry.resolve("deepseek", "deepseek-reasoner");

    let response = InternalResponse {
        content_blocks: vec![
            RawContentBlock::Text("answer".to_string()),
            RawContentBlock::Thinking {
                thinking: "reasoning trace".to_string(),
                signature: Some("sig_abc123".to_string()),
            },
        ],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: Some(3),
        },
        finish_reason: Some("stop".to_string()),
    };

    let unified = interp.interpret_response(response);
    // Should have Text + Thinking blocks
    assert_eq!(unified.content_blocks.len(), 2, "should produce 2 blocks");

    // Thinking block should carry the signature
    match &unified.content_blocks[1] {
        closeclaw_llm::types::ContentBlock::Thinking { signature, .. } => {
            assert_eq!(
                signature.as_deref(),
                Some("sig_abc123"),
                "signature should be forwarded"
            );
        }
        other => panic!("expected Thinking block, got {:?}", other),
    }
}

#[test]
fn test_deepseek_plugin_injects_reasoning_effort() {
    let (_, _, pipeline) = assemble_for_provider("deepseek");
    assert_eq!(pipeline.len(), 1, "deepseek pipeline should have 1 plugin");

    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::High;
    pipeline.before_request(&mut req);

    let effort = req.extra_body.get("reasoning_effort");
    assert!(
        effort.is_some(),
        "DeepSeekPlugin should inject reasoning_effort"
    );
    assert_eq!(
        effort.unwrap(),
        &serde_json::Value::String("high".to_string())
    );
}

#[test]
fn test_deepseek_plugin_effort_levels() {
    let (_, _, pipeline) = assemble_for_provider("deepseek");

    // Low
    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Low;
    pipeline.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort").unwrap(),
        &serde_json::Value::String("low".to_string())
    );

    // Medium → "base"
    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Medium;
    pipeline.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort").unwrap(),
        &serde_json::Value::String("base".to_string())
    );

    // Max → downgraded to High → "high"
    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Max;
    pipeline.before_request(&mut req);
    assert_eq!(
        req.extra_body.get("reasoning_effort").unwrap(),
        &serde_json::Value::String("high".to_string())
    );
    assert_eq!(
        req.reasoning_level,
        ReasoningLevel::High,
        "Max should be downgraded to High"
    );
}

// ── 3. GLM: OpenAiProtocol + GlmInterpreter + GlmPlugin ───────────────────

#[test]
fn test_glm_uses_openai_protocol() {
    let (protocol, _, _) = assemble_for_provider("glm");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "openai",
        "glm should use OpenAI protocol"
    );
}

#[test]
fn test_glm_interpreter_resolves_for_all_models() {
    let (_, registry, _) = assemble_for_provider("glm");
    let interp = registry.resolve("glm", "glm-4");
    assert_eq!(interp.name(), "glm", "glm/* glob should match glm-4");
}

#[test]
fn test_glm_interpreter_reasoning_to_text() {
    use closeclaw_llm::types::{InternalResponse, RawContentBlock, RawUsage};

    let (_, registry, _) = assemble_for_provider("glm");
    let interp = registry.resolve("glm", "glm-4");

    // Empty text + reasoning_content → should become Text block
    let response = InternalResponse {
        content_blocks: vec![RawContentBlock::Thinking {
            thinking: "glm reasoning".to_string(),
            signature: None,
        }],
        usage: RawUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: Some(15),
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
        finish_reason: Some("stop".to_string()),
    };

    let unified = interp.interpret_response(response);
    assert_eq!(unified.content_blocks.len(), 1, "should produce 1 block");
    match &unified.content_blocks[0] {
        closeclaw_llm::types::ContentBlock::Text(text) => {
            assert_eq!(text, "glm reasoning");
        }
        other => panic!("expected Text block, got {:?}", other),
    }
}

#[test]
fn test_glm_plugin_injects_thinking_type() {
    let (_, _, pipeline) = assemble_for_provider("glm");
    assert_eq!(pipeline.len(), 1, "glm pipeline should have 1 plugin");

    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Medium;
    pipeline.before_request(&mut req);

    let thinking = req.extra_body.get("thinking");
    assert!(thinking.is_some(), "GlmPlugin should inject thinking type");
    assert_eq!(thinking.unwrap(), &serde_json::json!({"type": "enabled"}));
}

#[test]
fn test_glm_plugin_disabled_for_low() {
    let (_, _, pipeline) = assemble_for_provider("glm");

    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Low;
    pipeline.before_request(&mut req);

    let thinking = req.extra_body.get("thinking").unwrap();
    assert_eq!(thinking, &serde_json::json!({"type": "disabled"}));
}

#[test]
fn test_glm_plugin_max_downgrades_to_high() {
    let (_, _, pipeline) = assemble_for_provider("glm");

    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::Max;
    pipeline.before_request(&mut req);

    assert_eq!(req.reasoning_level, ReasoningLevel::High);
    assert_eq!(
        req.extra_body.get("thinking").unwrap(),
        &serde_json::json!({"type": "enabled"})
    );
}

// ── 4. Mimo: OpenAiProtocol + DefaultInterpreter + empty pipeline ──────────

#[test]
fn test_mimo_uses_openai_protocol() {
    let (protocol, _, _) = assemble_for_provider("mimo");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "openai",
        "mimo should use OpenAI protocol"
    );
}

#[test]
fn test_mimo_interpreter_is_default() {
    let (_, registry, _) = assemble_for_provider("mimo");
    let interp = registry.resolve("mimo", "mimo-v2");
    assert_eq!(
        interp.name(),
        "default",
        "mimo should resolve to DefaultInterpreter"
    );
}

#[test]
fn test_mimo_pipeline_is_empty() {
    let (_, _, pipeline) = assemble_for_provider("mimo");
    assert!(
        pipeline.is_empty(),
        "mimo pipeline should be empty (no plugins)"
    );
}

#[test]
fn test_mimo_empty_pipeline_does_not_modify_request() {
    let (_, _, pipeline) = assemble_for_provider("mimo");
    let mut req = make_request();
    req.reasoning_level = ReasoningLevel::High;
    pipeline.before_request(&mut req);
    assert!(
        req.extra_body.is_empty(),
        "empty pipeline should not inject anything"
    );
}

// ── 5. Unknown provider → default branch ───────────────────────────────────

#[test]
fn test_unknown_provider_uses_openai_protocol() {
    let (protocol, _, _) = assemble_for_provider("some-random-provider");
    assert_eq!(
        protocol.protocol_id().as_str(),
        "openai",
        "unknown provider should default to OpenAI protocol"
    );
}

#[test]
fn test_unknown_provider_interpreter_is_default() {
    let (_, registry, _) = assemble_for_provider("some-random-provider");
    let interp = registry.resolve("some-random-provider", "any-model");
    assert_eq!(
        interp.name(),
        "default",
        "unknown provider should resolve to DefaultInterpreter"
    );
}

#[test]
fn test_unknown_provider_pipeline_is_empty() {
    let (_, _, pipeline) = assemble_for_provider("some-random-provider");
    assert!(
        pipeline.is_empty(),
        "unknown provider pipeline should be empty"
    );
}

#[test]
fn test_unknown_provider_default_branch_does_not_panic() {
    // The default branch must not panic for any provider_id string
    let (_, _, pipeline) = assemble_for_provider("not-even-real");
    let mut req = make_request();
    pipeline.before_request(&mut req);
    pipeline.after_response(&mut closeclaw_llm::types::UnifiedResponse {
        content_blocks: vec![],
        usage: closeclaw_llm::types::UnifiedUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: None,
            reasoning_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        },
        finish_reason: None,
        retry_attempts: 0,
    });
}

// ── 6. Smoke: all known providers iterate without panic ─────────────────────

#[test]
fn test_all_known_providers_assemble_without_panic() {
    let known_providers = ["minimax", "deepseek", "glm", "mimo"];
    for provider_id in &known_providers {
        let (protocol, registry, pipeline) = assemble_for_provider(provider_id);

        // Verify protocol produces a valid request
        let mut req = make_request();
        req.model = format!("{}-test-model", provider_id);
        let _ = protocol.build_request(&req);

        // Verify interpreter resolves without panic
        let interp = registry.resolve(provider_id, "any-model");
        let _ = interp.name();

        // Verify pipeline hooks don't panic
        pipeline.before_request(&mut req);
    }
}

#[test]
fn test_unknown_provider_assembles_without_panic() {
    let (protocol, registry, pipeline) = assemble_for_provider("completely-unknown");

    let mut req = make_request();
    req.model = "unknown-model".to_string();
    let _ = protocol.build_request(&req);

    let interp = registry.resolve("completely-unknown", "unknown-model");
    let _ = interp.name();

    pipeline.before_request(&mut req);
}

// ── 7. Cross-provider isolation: interpreters don't cross-match ────────────

#[test]
fn test_interpreter_isolation_minimax_does_not_match_deepseek() {
    let (_, minimax_reg, _) = assemble_for_provider("minimax");
    let interp = minimax_reg.resolve("deepseek", "deepseek-reasoner");
    assert_ne!(
        interp.name(),
        "minimax",
        "minimax registry should NOT resolve deepseek models"
    );
    assert_eq!(
        interp.name(),
        "default",
        "deepseek model in minimax registry should fall back to default"
    );
}

#[test]
fn test_interpreter_isolation_deepseek_does_not_match_glm() {
    let (_, deepseek_reg, _) = assemble_for_provider("deepseek");
    let interp = deepseek_reg.resolve("glm", "glm-4");
    assert_ne!(
        interp.name(),
        "deepseek",
        "deepseek registry should NOT resolve glm models"
    );
    assert_eq!(interp.name(), "default");
}

#[test]
fn test_interpreter_isolation_glm_does_not_match_mimo() {
    let (_, glm_reg, _) = assemble_for_provider("glm");
    let interp = glm_reg.resolve("mimo", "mimo-v2");
    assert_ne!(
        interp.name(),
        "glm",
        "glm registry should NOT resolve mimo models"
    );
    assert_eq!(interp.name(), "default");
}
