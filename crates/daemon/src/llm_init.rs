//! LLM provider registration helpers

use super::*;
use closeclaw_config::providers::CredentialsProvider;
use closeclaw_llm::call_chain;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;
use closeclaw_llm::LLMRegistry;
use std::collections::HashMap;

type DynProvider = Arc<dyn closeclaw_llm::provider::Provider>;
type FactoryFn = fn(String) -> DynProvider;

fn openai_factory(k: String) -> DynProvider {
    Arc::new(closeclaw_llm::openai::OpenAIProvider::new(k))
}
fn anthropic_factory(k: String) -> DynProvider {
    Arc::new(closeclaw_llm::anthropic::AnthropicProvider::new(k))
}
fn minimax_factory(k: String) -> DynProvider {
    Arc::new(closeclaw_llm::minimax::MiniMaxProvider::new(k))
}
fn mimo_factory(k: String) -> DynProvider {
    Arc::new(closeclaw_llm::mimo::MimoProvider::new(k))
}

struct ProviderSpec {
    name: &'static str,
    env_key: &'static str,
    display_name: &'static str,
    factory: FactoryFn,
}

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        name: "openai",
        env_key: "OPENAI_API_KEY",
        display_name: "OpenAI",
        factory: openai_factory,
    },
    ProviderSpec {
        name: "anthropic",
        env_key: "ANTHROPIC_API_KEY",
        display_name: "Anthropic",
        factory: anthropic_factory,
    },
    ProviderSpec {
        name: "minimax",
        env_key: "MINIMAX_API_KEY",
        display_name: "MiniMax",
        factory: minimax_factory,
    },
    ProviderSpec {
        name: "mimo",
        env_key: "MIMO_API_KEY",
        display_name: "MiMo",
        factory: mimo_factory,
    },
];

/// Resolve an API key for `provider_name` with precedence:
/// credential file → `env_overrides` map → environment variable.
fn resolve_api_key(
    creds_provider: &CredentialsProvider,
    provider_name: &str,
    env_key: &str,
    env_overrides: &HashMap<&str, &str>,
) -> Option<String> {
    creds_provider
        .get_api_key(provider_name)
        .or_else(|| env_overrides.get(env_key).map(|s| s.to_string()))
        .or_else(|| std::env::var(env_key).ok())
        .filter(|k| !k.is_empty())
}

/// Try to register a provider in `registry` with the given factory.
/// Logs success on registration, does nothing if no key is available.
async fn try_register(
    registry: &LLMRegistry,
    creds_provider: &CredentialsProvider,
    spec: &ProviderSpec,
    env_overrides: &HashMap<&str, &str>,
) {
    if let Some(api_key) = resolve_api_key(creds_provider, spec.name, spec.env_key, env_overrides) {
        let provider = (spec.factory)(api_key);
        registry.register(spec.name.to_string(), provider).await;
        info!("{} provider registered", spec.display_name);
    }
}

impl Daemon {
    /// Initialize the LLM registry with credentials from config_dir or env vars.
    ///
    /// For each provider (openai, anthropic, minimax, mimo):
    /// 1. Try to load api_key from `config_dir/config/credentials/<provider>.json`
    /// 2. Fall back to `env_overrides` map
    /// 3. Fall back to the corresponding env var if neither has it
    pub(crate) async fn init_llm_registry(
        config_dir: &Path,
        env_overrides: &HashMap<&str, &str>,
    ) -> (Arc<LLMRegistry>, Arc<UnifiedFallbackClient>) {
        let registry = Arc::new(LLMRegistry::new());
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
        for spec in PROVIDERS {
            try_register(&registry, &creds_provider, spec, env_overrides).await;
        }
        let fallback_client = call_chain::build_fallback_client(&registry).await;
        info!(
            chain_len = fallback_client.chain().len(),
            "LLM fallback client built in layer 2"
        );
        (registry, fallback_client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Helper: write a fake credentials JSON file for a given provider.
    fn write_fake_creds(dir: &std::path::Path, provider: &str, api_key: &str) {
        let creds_dir = dir.join("credentials");
        std::fs::create_dir_all(&creds_dir).unwrap();
        let file = creds_dir.join(format!("{}.json", provider));
        let content = serde_json::json!({
            "provider": provider,
            "apiKey": api_key,
        });
        std::fs::write(&file, content.to_string()).unwrap();
    }

    /// init_llm_registry with fake credentials returns a fallback client
    /// whose chain length matches the number of registered providers.
    #[tokio::test]
    async fn init_llm_registry_returns_fallback_with_correct_chain_length() {
        let dir = tempfile::tempdir().unwrap();
        write_fake_creds(dir.path(), "openai", "sk-fake-openai");
        write_fake_creds(dir.path(), "anthropic", "sk-fake-anthropic");

        let env_overrides = HashMap::new();
        let (registry, fallback_client) =
            Daemon::init_llm_registry(dir.path(), &env_overrides).await;

        let provider_ids = registry.list().await;
        assert_eq!(provider_ids.len(), 2, "registry should have 2 providers");
        assert!(provider_ids.contains(&"openai".to_string()));
        assert!(provider_ids.contains(&"anthropic".to_string()));

        assert_eq!(
            fallback_client.chain().len(),
            2,
            "fallback chain length must equal registry provider count"
        );
    }

    /// init_llm_registry with no credentials returns an empty fallback
    /// client (chain length 0) without panicking.
    #[tokio::test]
    async fn init_llm_registry_no_credentials_returns_empty_chain() {
        let dir = tempfile::tempdir().unwrap();
        let env_overrides = HashMap::new();
        let (registry, fallback_client) =
            Daemon::init_llm_registry(dir.path(), &env_overrides).await;

        let provider_ids = registry.list().await;
        assert!(provider_ids.is_empty(), "no providers should be registered");
        assert_eq!(
            fallback_client.chain().len(),
            0,
            "fallback chain should be empty when no credentials"
        );
    }

    /// init_llm_registry with env overrides registers the providers
    /// even without credential files.
    #[tokio::test]
    async fn init_llm_registry_env_overrides_register_providers() {
        let dir = tempfile::tempdir().unwrap();
        let mut env_overrides = HashMap::new();
        env_overrides.insert("OPENAI_API_KEY", "sk-env-openai");
        env_overrides.insert("MINIMAX_API_KEY", "sk-env-minimax");

        let (registry, fallback_client) =
            Daemon::init_llm_registry(dir.path(), &env_overrides).await;

        let provider_ids = registry.list().await;
        assert!(provider_ids.contains(&"openai".to_string()));
        assert!(provider_ids.contains(&"minimax".to_string()));
        assert_eq!(fallback_client.chain().len(), 2);
    }

    /// init_llm_registry with non-existent credentials dir returns
    /// empty chain (does not panic).
    #[tokio::test]
    async fn init_llm_registry_nonexistent_creds_dir_returns_empty_chain() {
        let nonexistent = std::path::PathBuf::from("/tmp/nonexistent_creds_dir_");
        let env_overrides = HashMap::new();
        let (registry, fallback_client) =
            Daemon::init_llm_registry(&nonexistent, &env_overrides).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }
}
