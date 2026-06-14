//! LLM provider registration helpers

use std::collections::HashMap;

use super::*;
use crate::config::providers::CredentialsProvider;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::minimax::MiniMaxProvider;
use crate::llm::openai::OpenAIProvider;
use crate::llm::LLMRegistry;

impl Daemon {
    /// Initialize the LLM registry with credentials from config_dir or env vars.
    ///
    /// For each provider (openai, anthropic, minimax):
    /// 1. Try to load api_key from `config_dir/config/credentials/<provider>.json`
    /// 2. Fall back to the corresponding env var if the file does not have it
    #[allow(dead_code)] // only invoked from `#[cfg(test)] mod unit_tests`
    pub(crate) async fn init_llm_registry(config_dir: &Path) -> Arc<LLMRegistry> {
        Self::init_llm_registry_with_env(config_dir, &HashMap::new()).await
    }

    /// Same as [`init_llm_registry`] but accepts an override map for env-var fallback.
    ///
    /// For each provider the lookup order is:
    /// 1. Credentials file
    /// 2. `env_overrides` map
    /// 3. Process environment variable
    #[allow(dead_code)] // only invoked from `#[cfg(test)] mod unit_tests`
    pub(crate) async fn init_llm_registry_with_env(
        config_dir: &Path,
        env_overrides: &HashMap<&str, &str>,
    ) -> Arc<LLMRegistry> {
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

        // Register OpenAI provider: credentials file first, then env var fallback
        let openai_key = creds_provider
            .get_api_key("openai")
            .or_else(|| {
                env_overrides
                    .get("OPENAI_API_KEY")
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            })
            .filter(|k| !k.is_empty());
        if let Some(api_key) = openai_key {
            let provider: Arc<dyn crate::llm::provider::Provider> =
                Arc::new(OpenAIProvider::new(api_key));
            registry.register("openai".to_string(), provider).await;
            info!("OpenAI provider registered");
        }

        // Register Anthropic provider: credentials file first, then env var fallback
        let anthropic_key = creds_provider
            .get_api_key("anthropic")
            .or_else(|| {
                env_overrides
                    .get("ANTHROPIC_API_KEY")
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            })
            .filter(|k| !k.is_empty());
        if let Some(api_key) = anthropic_key {
            let provider: Arc<dyn crate::llm::provider::Provider> =
                Arc::new(AnthropicProvider::new(api_key.clone()));
            registry.register("anthropic".to_string(), provider).await;
            info!("Anthropic provider registered");
        }

        // Register MiniMax provider: credentials file first, then env var fallback
        let minimax_key = creds_provider
            .get_api_key("minimax")
            .or_else(|| {
                env_overrides
                    .get("MINIMAX_API_KEY")
                    .map(|s| s.to_string())
                    .or_else(|| std::env::var("MINIMAX_API_KEY").ok())
            })
            .filter(|k| !k.is_empty());
        if let Some(api_key) = minimax_key {
            let provider: Arc<dyn crate::llm::provider::Provider> =
                Arc::new(MiniMaxProvider::new(api_key));
            registry.register("minimax".to_string(), provider).await;
            info!("MiniMax provider registered");
        }

        registry
    }
}
