//! LLM provider registration helpers

#![allow(deprecated)]

use super::*;
use crate::config::providers::CredentialsProvider;
use crate::llm::anthropic::AnthropicProvider;
use crate::llm::legacy::legacy_provider::LegacyProviderBridge;
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

        let client = reqwest::Client::new();
        let empty_headers = reqwest::header::HeaderMap::new();

        // Register OpenAI provider: credentials file first, then env var fallback
        let openai_key = creds_provider
            .get_api_key("openai")
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|k| !k.is_empty());
        if let Some(api_key) = openai_key {
            let bridge = Arc::new(LegacyProviderBridge::new(
                OpenAIProvider::new(api_key.clone()),
                "https://api.openai.com/v1".to_string(),
                api_key,
                vec![],
                client.clone(),
                empty_headers.clone(),
            ));
            registry.register("openai".to_string(), bridge).await;
            info!("OpenAI provider registered");
        }

        // Register Anthropic provider: credentials file first, then env var fallback
        let anthropic_key = creds_provider
            .get_api_key("anthropic")
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .filter(|k| !k.is_empty());
        if let Some(api_key) = anthropic_key {
            let bridge = Arc::new(LegacyProviderBridge::new(
                AnthropicProvider::new(api_key.clone()),
                "https://api.anthropic.com".to_string(),
                api_key,
                vec![],
                client.clone(),
                empty_headers.clone(),
            ));
            registry.register("anthropic".to_string(), bridge).await;
            info!("Anthropic provider registered");
        }

        // Register MiniMax provider: credentials file first, then env var fallback
        let minimax_key = creds_provider
            .get_api_key("minimax")
            .or_else(|| std::env::var("MINIMAX_API_KEY").ok())
            .filter(|k| !k.is_empty());
        if let Some(api_key) = minimax_key {
            let bridge = Arc::new(LegacyProviderBridge::new(
                MiniMaxProvider::new(api_key.clone()),
                "https://api.minimax.chat".to_string(),
                api_key,
                vec![],
                client.clone(),
                empty_headers.clone(),
            ));
            registry.register("minimax".to_string(), bridge).await;
            info!("MiniMax provider registered");
        }

        registry
    }
}
