//! Shared LLM call chain construction.
//!
//! Provides [`assemble_llm_components`] for per-provider protocol /
//! interpreter / plugin mapping, and [`build_chain_entries`] /
//! [`build_fallback_client`] to assemble the full call chain from a
//! registry. Used by both daemon (layer 2) and CLI.
//!
//! See also: `docs/design/llm/README.md` § 五层架构

use crate::cache_adapter;
use crate::client::UnifiedChatClient;
use crate::interpreter::InterpreterRegistry;
use crate::plugin::PluginPipeline;
use crate::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use crate::retry::CooldownManager;
use crate::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use crate::LLMRegistry;
use std::sync::Arc;

/// Assemble per-provider protocol, interpreter, and plugin pipeline.
///
/// Returns `(protocol, interpreter_registry, plugin_pipeline)` for the
/// given `provider_id`. Unknown providers receive OpenAI protocol,
/// `DefaultInterpreter`, and an empty pipeline.
pub fn assemble_llm_components(
    provider_id: &str,
) -> (Arc<dyn ChatProtocol>, InterpreterRegistry, PluginPipeline) {
    use crate::plugin::PluginPipeline;
    match provider_id {
        // AnthropicInterpreter needed because DefaultInterpreter doesn't:
        // - merge empty-text + non-empty-thinking into Text block
        // - handle signature-only Thinking blocks from signature_delta
        "anthropic" => (
            Arc::new(AnthropicProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::new(vec![(Box::new(crate::AnthropicInterpreter), "anthropic/*")]),
            PluginPipeline::new().add(Box::new(crate::AnthropicPlugin)),
        ),
        "minimax" => (
            Arc::new(AnthropicProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::new(vec![(Box::new(crate::MinimaxInterpreter), "minimax/*")]),
            PluginPipeline::new()
                .add(Box::new(crate::MiniMaxM3Plugin))
                .add(Box::new(crate::MiniMaxM2Plugin)),
        ),
        "deepseek" => (
            Arc::new(AnthropicProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::new(vec![(Box::new(crate::DeepSeekInterpreter), "deepseek/*")]),
            PluginPipeline::new().add(Box::new(crate::DeepSeekPlugin)),
        ),
        "glm" => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::new(vec![(Box::new(crate::GlmInterpreter), "glm/*")]),
            PluginPipeline::new().add(Box::new(crate::GlmPlugin)),
        ),
        "mimo" => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::new(vec![(Box::new(crate::MimoInterpreter), "mimo/*")]),
            PluginPipeline::new(),
        ),
        _ => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::default(),
            PluginPipeline::new(),
        ),
    }
}

/// Build chain entries from every provider registered in `registry`.
///
/// For each registered provider, assembles protocol / interpreter / plugin
/// via [`assemble_llm_components`], wraps the result in a
/// [`UnifiedChatClient`] with the appropriate cache adapter, and returns
/// the full list of [`ChainEntry`]s.
pub async fn build_chain_entries(registry: &Arc<LLMRegistry>) -> Vec<ChainEntry> {
    let provider_ids = registry.list().await;
    let mut entries = Vec::with_capacity(provider_ids.len());
    for provider_id in &provider_ids {
        if let Some(provider) = registry.get(provider_id).await {
            let (protocol, interpreter, plugin) = assemble_llm_components(provider_id.as_str());
            let cache = cache_adapter::for_provider(provider_id);
            let client = UnifiedChatClient::new(provider, protocol, interpreter, plugin, cache);
            entries.push(ChainEntry {
                provider_id: provider_id.clone(),
                model_id: provider_id.clone(),
                client: Arc::new(client),
            });
        }
    }
    entries
}

/// Build a complete [`UnifiedFallbackClient`] from `registry`.
///
/// Constructs [`ChainEntry`]s via [`build_chain_entries`] and wraps them
/// in a fallback client with a fresh [`CooldownManager`]. The returned
/// client is ready for injection into SessionManager / ActiveSearcher.
pub async fn build_fallback_client(registry: &Arc<LLMRegistry>) -> Arc<UnifiedFallbackClient> {
    let entries = build_chain_entries(registry).await;
    let cooldown = Arc::new(CooldownManager::new());
    Arc::new(UnifiedFallbackClient::new(entries, cooldown))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use crate::stub::StubProvider;

    fn stub_provider() -> Arc<dyn Provider> {
        Arc::new(StubProvider::new())
    }

    #[test]
    fn assemble_unknown_provider_uses_openai_and_default_interpreter() {
        let (protocol, interpreter, _plugin) = assemble_llm_components("unknown");
        assert_eq!(protocol.protocol_id().as_str(), "openai");
        // DefaultInterpreter should be registered — resolve returns it
        // for any (provider_id, model) pair.
        let resolved = interpreter.resolve("unknown", "any-model");
        assert_eq!(
            resolved.name(),
            "default",
            "unknown provider should use DefaultInterpreter"
        );
    }

    #[test]
    fn assemble_minimax_uses_anthropic_protocol() {
        let (protocol, interpreter, plugin) = assemble_llm_components("minimax");
        assert_eq!(protocol.protocol_id().as_str(), "anthropic");
        let resolved = interpreter.resolve("minimax", "minimax/some-model");
        assert_eq!(
            resolved.name(),
            "minimax",
            "MinimaxInterpreter should resolve minimax/* models"
        );
        assert!(!plugin.is_empty(), "minimax should have plugins");
    }

    #[test]
    fn assemble_anthropic_uses_anthropic_protocol() {
        let (protocol, interpreter, plugin) = assemble_llm_components("anthropic");
        assert_eq!(protocol.protocol_id().as_str(), "anthropic");
        assert!(!plugin.is_empty(), "anthropic should have plugins");
        let resolved = interpreter.resolve("anthropic", "anthropic/claude-sonnet-4-20250514");
        assert_eq!(
            resolved.name(),
            "anthropic",
            "AnthropicInterpreter should resolve anthropic/* models"
        );
    }

    #[test]
    fn assemble_deepseek_uses_anthropic_protocol() {
        let (protocol, _, _) = assemble_llm_components("deepseek");
        assert_eq!(protocol.protocol_id().as_str(), "anthropic");
    }

    #[test]
    fn assemble_glm_uses_openai_protocol() {
        let (protocol, _, _) = assemble_llm_components("glm");
        assert_eq!(protocol.protocol_id().as_str(), "openai");
    }

    #[test]
    fn assemble_mimo_uses_openai_protocol() {
        let (protocol, _, _) = assemble_llm_components("mimo");
        assert_eq!(protocol.protocol_id().as_str(), "openai");
    }

    #[tokio::test]
    async fn build_chain_entries_empty_registry_returns_empty() {
        let registry = Arc::new(LLMRegistry::new());
        let entries = build_chain_entries(&registry).await;
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn build_chain_entries_one_provider_returns_one_entry() {
        let registry = Arc::new(LLMRegistry::new());
        registry.register("stub".to_string(), stub_provider()).await;
        let entries = build_chain_entries(&registry).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider_id, "stub");
        assert_eq!(entries[0].model_id, "stub");
    }

    #[tokio::test]
    async fn build_chain_entries_multiple_providers() {
        let registry = Arc::new(LLMRegistry::new());
        registry.register("a".to_string(), stub_provider()).await;
        registry.register("b".to_string(), stub_provider()).await;
        registry.register("c".to_string(), stub_provider()).await;
        let entries = build_chain_entries(&registry).await;
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn build_fallback_client_empty_registry_returns_usable_client() {
        let registry = Arc::new(LLMRegistry::new());
        let client = build_fallback_client(&registry).await;
        // Chain should be empty but cooldown exists — calling chat on
        // empty chain returns exhaustion error (not panic).
        assert_eq!(client.chain().len(), 0);
    }
}
