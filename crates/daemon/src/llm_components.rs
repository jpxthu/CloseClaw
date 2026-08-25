//! Per-provider LLM component assembly (protocol, interpreter, plugin).
//!
//! Extracted from `lifecycle.rs` to keep that file focused on daemon
//! lifecycle orchestration.
//!
//! See also: `docs/design/llm/README.md` § 五层架构

use std::sync::Arc;

/// Assemble protocol, interpreter, and plugin per provider (design doc).
/// Shared by production wiring and `lifecycle_assembly_tests`.
pub(crate) fn assemble_llm_components(
    provider_id: &str,
) -> (
    Arc<dyn closeclaw_llm::protocol::ChatProtocol>,
    closeclaw_llm::InterpreterRegistry,
    closeclaw_llm::PluginPipeline,
) {
    use closeclaw_llm::plugin::PluginPipeline;
    use closeclaw_llm::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
    match provider_id {
        "minimax" => (
            Arc::new(AnthropicProtocol::new()) as Arc<dyn ChatProtocol>,
            closeclaw_llm::InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::MinimaxInterpreter),
                "minimax/*",
            )]),
            PluginPipeline::new()
                .add(Box::new(closeclaw_llm::MiniMaxM3Plugin))
                .add(Box::new(closeclaw_llm::MiniMaxM2Plugin)),
        ),
        "deepseek" => (
            Arc::new(AnthropicProtocol::new()) as Arc<dyn ChatProtocol>,
            closeclaw_llm::InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::DeepSeekInterpreter),
                "deepseek/*",
            )]),
            PluginPipeline::new().add(Box::new(closeclaw_llm::DeepSeekPlugin)),
        ),
        "glm" => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            closeclaw_llm::InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::GlmInterpreter),
                "glm/*",
            )]),
            PluginPipeline::new().add(Box::new(closeclaw_llm::GlmPlugin)),
        ),
        "mimo" => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            closeclaw_llm::InterpreterRegistry::new(vec![(
                Box::new(closeclaw_llm::MimoInterpreter),
                "mimo/*",
            )]),
            PluginPipeline::new(),
        ),
        // all others: OpenAI protocol, DefaultInterpreter, empty pipeline
        _ => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            closeclaw_llm::InterpreterRegistry::default(),
            PluginPipeline::new(),
        ),
    }
}
