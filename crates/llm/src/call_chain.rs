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
use crate::provider::Provider;
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
            PluginPipeline::new().add(Box::new(crate::MimoPlugin)),
        ),
        _ => (
            Arc::new(OpenAiProtocol::new()) as Arc<dyn ChatProtocol>,
            InterpreterRegistry::default(),
            PluginPipeline::new(),
        ),
    }
}

/// Construct the vendor provider implementation for `provider_id` with an
/// optional custom `base_url` (the vendor's default endpoint when absent
/// or empty).
///
/// Single source of truth for the models.json provider id → vendor
/// implementation mapping, kept next to [`assemble_llm_components`] so the
/// constructor and protocol / interpreter / plugin tables evolve together.
/// Returns `None` for provider ids without a vendor implementation —
/// callers decide how to report the skip.
pub fn build_vendor_provider(
    provider_id: &str,
    api_key: &str,
    base_url: Option<&str>,
) -> Option<Arc<dyn Provider>> {
    let url = base_url.filter(|url| !url.is_empty());
    let key = api_key.to_string();
    let provider: Arc<dyn Provider> = match provider_id {
        "openai" => match url {
            Some(url) => Arc::new(crate::OpenAIProvider::new_with_base_url(key, url)),
            None => Arc::new(crate::OpenAIProvider::new(key)),
        },
        "anthropic" => match url {
            Some(url) => Arc::new(crate::AnthropicProvider::new_with_base_url(key, url)),
            None => Arc::new(crate::AnthropicProvider::new(key)),
        },
        "minimax" => match url {
            Some(url) => Arc::new(crate::MiniMaxProvider::with_base_url(key, url.to_string())),
            None => Arc::new(crate::MiniMaxProvider::new(key)),
        },
        "mimo" => match url {
            Some(url) => Arc::new(crate::MimoProvider::with_base_url(key, url)),
            None => Arc::new(crate::MimoProvider::new(key)),
        },
        "glm" => match url {
            Some(url) => Arc::new(crate::GlmProvider::with_base_url(key, url.to_string())),
            None => Arc::new(crate::GlmProvider::new(key)),
        },
        "deepseek" => match url {
            Some(url) => Arc::new(crate::DeepSeekProvider::with_base_url(key, url.to_string())),
            None => Arc::new(crate::DeepSeekProvider::new(key)),
        },
        "volcengine" => match url {
            Some(url) => Arc::new(crate::VolcEngineProvider::with_base_url(
                key,
                url.to_string(),
            )),
            None => Arc::new(crate::VolcEngineProvider::new(key)),
        },
        _ => return None,
    };
    Some(provider)
}

/// Build a single fallback-chain entry for `provider`.
///
/// The single assembly point for [`ChainEntry`]: protocol / interpreter /
/// plugin via [`assemble_llm_components`], wrapped in a
/// [`UnifiedChatClient`] with the provider's cache adapter. Consumed by
/// both [`build_chain_entries`] (registry-driven, used by CLI and the
/// fallback-client builder) and the daemon's models.json-driven chain
/// construction, so this wiring lives in exactly one place. The caller
/// supplies `provider_id` (keys protocol/interpreter/plugin/cache
/// selection) and `model_id` (what reaches the wire) independently.
pub fn build_chain_entry(
    provider: Arc<dyn Provider>,
    provider_id: &str,
    model_id: &str,
) -> ChainEntry {
    let (protocol, interpreter, plugin) = assemble_llm_components(provider_id);
    let cache = cache_adapter::for_provider(provider_id);
    let client = UnifiedChatClient::new(provider, protocol, interpreter, plugin, cache);
    ChainEntry {
        provider_id: provider_id.to_string(),
        model_id: model_id.to_string(),
        client: Arc::new(client),
    }
}

/// Build chain entries from every provider registered in `registry`.
///
/// Delegates each entry to the single assembly point
/// [`build_chain_entry`], keying both ids on the provider id.
pub async fn build_chain_entries(registry: &Arc<LLMRegistry>) -> Vec<ChainEntry> {
    let provider_ids = registry.list().await;
    let mut entries = Vec::with_capacity(provider_ids.len());
    for provider_id in &provider_ids {
        if let Some(provider) = registry.get(provider_id).await {
            entries.push(build_chain_entry(provider, provider_id, provider_id));
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

    /// Every vendor implemented by this crate constructs with the custom
    /// base_url reflected in `Provider::base_url`.
    #[test]
    fn build_vendor_provider_all_vendors_use_configured_base_url() {
        for id in [
            "openai",
            "anthropic",
            "minimax",
            "mimo",
            "glm",
            "deepseek",
            "volcengine",
        ] {
            let url = format!("http://127.0.0.1:9/{id}");
            let provider = build_vendor_provider(id, "key", Some(&url)).expect(id);
            assert_eq!(provider.base_url(), url, "{id}");
        }
    }

    /// Absent and empty base_url both fall back to the vendor default
    /// endpoint (each vendor's `new` delegates to its base-url constructor
    /// with the same default, so both paths agree).
    #[test]
    fn build_vendor_provider_default_base_url_when_absent_or_empty() {
        for id in [
            "openai",
            "anthropic",
            "minimax",
            "mimo",
            "glm",
            "deepseek",
            "volcengine",
        ] {
            let default_url = build_vendor_provider(id, "key", None).expect(id);
            let empty_url = build_vendor_provider(id, "key", Some("")).expect(id);
            assert_eq!(default_url.base_url(), empty_url.base_url(), "{id}");
            assert!(
                !default_url.base_url().is_empty(),
                "{id} vendor default must be non-empty"
            );
        }
    }

    /// Provider ids without a vendor implementation return `None` so the
    /// caller can report the skip (no silent fallthrough to a wrong vendor).
    #[test]
    fn build_vendor_provider_unknown_id_returns_none() {
        assert!(build_vendor_provider("acme", "key", Some("http://127.0.0.1:9")).is_none());
    }

    /// The single assembly point carries the caller's ids through
    /// unchanged (the daemon passes the real models.json model id).
    #[test]
    fn build_chain_entry_carries_provider_and_model_ids() {
        let entry = build_chain_entry(stub_provider(), "openai", "gpt-4o-basic");
        assert_eq!(entry.provider_id, "openai");
        assert_eq!(entry.model_id, "gpt-4o-basic");
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
