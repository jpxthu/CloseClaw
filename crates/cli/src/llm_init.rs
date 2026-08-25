//! LLM provider initialization for CLI chat.
//!
//! Loads credentials from the config directory and assembles the
//! LLM call chain (provider → protocol → interpreter → plugin → cache → fallback).

use std::path::Path;
use std::sync::Arc;

use closeclaw_config::providers::CredentialsProvider;
use closeclaw_config::ConfigProvider;
use closeclaw_llm::anthropic::AnthropicProvider;
use closeclaw_llm::cache_adapter;
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::mimo::MimoProvider;
use closeclaw_llm::minimax::MiniMaxProvider;
use closeclaw_llm::openai::OpenAIProvider;
use closeclaw_llm::protocol::{AnthropicProtocol, ChatProtocol, OpenAiProtocol};
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use closeclaw_llm::LLMRegistry;

/// Assemble protocol, interpreter, and plugin per provider.
///
/// Mirrors the daemon's `assemble_llm_components` for CLI chat.
fn assemble_llm_components(
    provider_id: &str,
) -> (
    Arc<dyn ChatProtocol>,
    closeclaw_llm::InterpreterRegistry,
    closeclaw_llm::PluginPipeline,
) {
    use closeclaw_llm::plugin::PluginPipeline;
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

async fn try_register(
    registry: &LLMRegistry,
    creds: &crate::llm_init::CredentialsProvider,
    id: &str,
    make: impl FnOnce(String) -> Arc<dyn closeclaw_llm::provider::Provider>,
) {
    if let Some(key) = creds.get_api_key(id) {
        registry.register(id.to_string(), make(key)).await;
        tracing::info!("{} provider registered", id);
    }
}

/// Initialize the LLM registry by loading credentials from config directory.
pub async fn init_llm_registry(config_dir: &Path) -> Arc<LLMRegistry> {
    let registry = Arc::new(LLMRegistry::new());

    // Load credentials from config/credentials/ directory
    let creds_dir = config_dir.join(CredentialsProvider::config_path());
    let creds_provider = match CredentialsProvider::load_from_dir(&creds_dir) {
        Ok(cp) => cp,
        Err(e) => {
            tracing::warn!(
                "failed to load credentials from '{}': {}",
                creds_dir.display(),
                e
            );
            CredentialsProvider::default()
        }
    };

    try_register(&registry, &creds_provider, "openai", |k| {
        Arc::new(OpenAIProvider::new(k)) as Arc<dyn closeclaw_llm::provider::Provider>
    })
    .await;
    try_register(&registry, &creds_provider, "anthropic", |k| {
        Arc::new(AnthropicProvider::new(k)) as Arc<dyn closeclaw_llm::provider::Provider>
    })
    .await;
    try_register(&registry, &creds_provider, "minimax", |k| {
        Arc::new(MiniMaxProvider::new(k)) as Arc<dyn closeclaw_llm::provider::Provider>
    })
    .await;
    try_register(&registry, &creds_provider, "mimo", |k| {
        Arc::new(MimoProvider::new(k)) as Arc<dyn closeclaw_llm::provider::Provider>
    })
    .await;

    registry
}

/// Create a unified fallback client from the LLM registry.
///
/// Assembles the full LLM call chain: provider → protocol → interpreter
/// → plugin → cache → unified fallback.
pub async fn create_fallback_client(registry: &Arc<LLMRegistry>) -> Arc<UnifiedFallbackClient> {
    let provider_ids = registry.list().await;
    let mut chain_entries: Vec<ChainEntry> = Vec::new();

    for provider_id in &provider_ids {
        if let Some(provider) = registry.get(provider_id).await {
            let (protocol, interpreter_registry, plugin_pipeline) =
                assemble_llm_components(provider_id);
            let cache = cache_adapter::for_provider(provider_id);

            let client = UnifiedChatClient::new(
                provider,
                protocol,
                interpreter_registry,
                plugin_pipeline,
                cache,
            );
            chain_entries.push(ChainEntry {
                provider_id: provider_id.clone(),
                model_id: provider_id.clone(),
                client: Arc::new(client),
            });
        }
    }

    let cooldown = Arc::new(CooldownManager::new());
    Arc::new(UnifiedFallbackClient::new(chain_entries, cooldown))
}
