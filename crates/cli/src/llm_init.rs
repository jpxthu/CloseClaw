//! LLM provider initialization for CLI chat.
//!
//! Loads credentials from the config directory and constructs the
//! LLM call chain by delegating to `closeclaw_llm::call_chain`.

use std::path::Path;
use std::sync::Arc;

use closeclaw_config::providers::CredentialsProvider;
use closeclaw_config::ConfigProvider;
use closeclaw_llm::anthropic::AnthropicProvider;
use closeclaw_llm::mimo::MimoProvider;
use closeclaw_llm::minimax::MiniMaxProvider;
use closeclaw_llm::openai::OpenAIProvider;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;

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
/// Thin wrapper around [`closeclaw_llm::call_chain::build_fallback_client`].
pub async fn create_fallback_client(registry: &Arc<LLMRegistry>) -> Arc<UnifiedFallbackClient> {
    closeclaw_llm::call_chain::build_fallback_client(registry).await
}
