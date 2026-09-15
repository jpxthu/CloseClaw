//! LLM provider registration helpers
//!
//! Layer 2 LLM Registry construction (design doc `docs/design/daemon/README.md`):
//! consume the models.json provider definitions and credentials already loaded
//! by [`ConfigManager`] (`<root>/config/credentials/` convention directory plus
//! `credential_path` references from models.json, merged with credential_path
//! priority) to register vendor providers and assemble the unified fallback
//! chain. Providers without a usable API key are skipped; an empty chain never
//! blocks startup.
//!
//! Chain entries carry the real model id from models.json — the fallback
//! client overrides `request.model` per entry, so this is what reaches the
//! wire. Protocol / interpreter / plugin selection stays keyed on the
//! provider id (`closeclaw_llm::call_chain::assemble_llm_components`).

use super::*;
use closeclaw_config::providers::models::ProviderConfig;
use closeclaw_config::providers::{CredentialsProvider, ModelsConfigData};
use closeclaw_llm::call_chain;
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::retry::CooldownManager;
use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};
use closeclaw_llm::LLMRegistry;

type DynProvider = Arc<dyn closeclaw_llm::provider::Provider>;

impl Daemon {
    /// Initialize the LLM registry and fallback chain from ConfigManager.
    ///
    /// For each provider defined in models.json:
    /// 1. Resolve the api key: ConfigManager credentials first
    ///    (`config/credentials/<provider>.json` + `credential_path`
    ///    references), then the `<PROVIDER>_API_KEY` environment variable;
    ///    providers without a usable key are skipped.
    /// 2. Construct the vendor provider with the configured `baseUrl`.
    /// 3. Register it under the models.json provider key and append one
    ///    fallback-chain entry per enabled model.
    ///
    /// No usable provider → empty chain, startup proceeds (design doc:
    /// LLM 能力缺失时系统仍正常启动).
    ///
    /// `env_lookup` is the `<PROVIDER>_API_KEY` fallback: production
    /// passes [`Self::process_env`] (`std::env::var`), tests inject a
    /// deterministic stub so outcomes never depend on host env vars.
    pub(crate) async fn init_llm_registry<F>(
        config_manager: &ConfigManager,
        env_lookup: F,
    ) -> (Arc<LLMRegistry>, Arc<UnifiedFallbackClient>)
    where
        F: Fn(&str) -> Option<String>,
    {
        let registry = Arc::new(LLMRegistry::new());
        let models = Self::load_models_config(config_manager);
        // Credentials固化 at ConfigManager::load: convention directory +
        // credential_path merge (credential_path wins on conflicts).
        let credentials = config_manager.credentials().unwrap_or_default();

        // Sorted provider ids → deterministic registry/chain order.
        let mut provider_ids: Vec<&String> = models.providers.keys().collect();
        provider_ids.sort();
        let mut chain_entries: Vec<ChainEntry> = Vec::new();
        for provider_id in provider_ids {
            let provider_cfg = &models.providers[provider_id];
            let Some(api_key) = Self::resolve_api_key(&credentials, provider_id, &env_lookup)
            else {
                info!(provider = %provider_id, "no credential available, provider skipped");
                continue;
            };
            let Some(provider) = call_chain::build_vendor_provider(
                provider_id,
                &api_key,
                provider_cfg.base_url.as_deref(),
            ) else {
                tracing::warn!(
                    provider = %provider_id,
                    "no vendor implementation for provider, skipped"
                );
                continue;
            };
            registry
                .register(provider_id.clone(), provider.clone())
                .await;
            info!(provider = %provider_id, "provider registered from models.json");
            chain_entries.extend(chain_entries_for(provider_id, &provider, provider_cfg));
        }

        let fallback_client = Arc::new(UnifiedFallbackClient::new(
            chain_entries,
            Arc::new(CooldownManager::new()),
        ));
        info!(
            chain_len = fallback_client.chain().len(),
            "LLM fallback client built in layer 2"
        );
        (registry, fallback_client)
    }

    /// Load models.json definitions from ConfigManager; an absent or
    /// unparsable section yields the empty default — non-blocking, but
    /// logged as WARN (config design: load failures never block startup).
    fn load_models_config(config_manager: &ConfigManager) -> ModelsConfigData {
        match config_manager.section(ConfigSection::Models) {
            Some(value) => {
                serde_json::from_value::<ModelsConfigData>(value).unwrap_or_else(|err| {
                    tracing::warn!(%err, "models.json parse failed — using empty model config");
                    ModelsConfigData::default()
                })
            }
            None => {
                tracing::warn!("models section missing after load — using empty model config");
                ModelsConfigData::default()
            }
        }
    }

    /// Production env lookup for the api-key fallback: read-only
    /// `std::env::var` (STANDARDS §7 — the forbidden write variants are
    /// not used here). Passed into [`Self::init_llm_registry`] as the
    /// `env_lookup` argument so tests can inject a deterministic stub
    /// instead of touching the real process environment
    /// (STANDARDS §7/§9).
    pub(crate) fn process_env(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// Resolve the api key for `provider_id`.
    ///
    /// Priority: ConfigManager credentials (convention directory +
    /// credential_path) → environment variable `<PROVIDER_ID>_API_KEY`
    /// via `env_lookup`. Empty keys are treated as absent.
    fn resolve_api_key(
        credentials: &CredentialsProvider,
        provider_id: &str,
        env_lookup: impl Fn(&str) -> Option<String>,
    ) -> Option<String> {
        credentials
            .get_api_key(provider_id)
            .or_else(|| env_lookup(&format!("{}_API_KEY", provider_id.to_uppercase())))
            .filter(|key| !key.is_empty())
    }
}

/// Build fallback-chain entries for one registered provider.
///
/// One entry per enabled model; `enabled` defaults to true when the flag
/// is omitted — a model listed in models.json is usable unless explicitly
/// disabled (`enabled: false`). This is the single semantics for the
/// field, mirrored by `closeclaw_config`'s `enabled_providers()`.
/// `model_id` carries the real model id from models.json so the outbound
/// request names the configured model.
fn chain_entries_for(
    provider_id: &str,
    provider: &DynProvider,
    provider_cfg: &ProviderConfig,
) -> Vec<ChainEntry> {
    let mut entries = Vec::new();
    for model in &provider_cfg.models {
        if model.enabled == Some(false) {
            continue;
        }
        let (protocol, interpreter, plugin) = call_chain::assemble_llm_components(provider_id);
        let cache = closeclaw_llm::cache_adapter::for_provider(provider_id);
        let client = UnifiedChatClient::new(provider.clone(), protocol, interpreter, plugin, cache);
        entries.push(ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model.id.clone(),
            client: Arc::new(client),
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Write the mandatory config skeleton into `dir` (the placeholder
    /// models.json is overwritten by `write_models_providers` afterwards).
    fn write_config_skeleton(dir: &Path) {
        crate::test_helpers::write_mandatory_configs(dir).expect("mandatory configs");
    }

    /// [`Daemon::init_llm_registry`] with a deterministic env: the api-key
    /// env fallback never resolves, so outcomes depend only on the
    /// fixture (models.json + credentials), never on host env vars.
    async fn init_registry_isolated(
        cm: &ConfigManager,
    ) -> (Arc<LLMRegistry>, Arc<UnifiedFallbackClient>) {
        Daemon::init_llm_registry(cm, |_| None).await
    }

    /// The env fallback resolves — used by the env-fallback test below.
    async fn init_registry_with_env(
        cm: &ConfigManager,
        env: &[(&str, &str)],
    ) -> (Arc<LLMRegistry>, Arc<UnifiedFallbackClient>) {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Daemon::init_llm_registry(cm, move |name| {
            env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        })
        .await
    }

    /// Normal path: models.json defines one provider + a credential file →
    /// registered, and the provider's base_url equals the configured value.
    /// The chain entry carries the real model id onto the wire.
    #[tokio::test]
    async fn init_llm_registry_registers_provider_with_configured_base_url() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "models": [{ "id": "gpt-4o-basic", "enabled": true }]
                }
            }),
        )
        .unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "openai", "sk-fake").unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        let provider = registry.get("openai").await.expect("openai registered");
        assert_eq!(provider.base_url(), "http://127.0.0.1:9/v1");

        let chain = fallback_client.chain();
        assert_eq!(chain.len(), 1, "one entry per configured model");
        assert_eq!(chain[0].provider_id, "openai");
        assert_eq!(chain[0].model_id, "gpt-4o-basic");
    }

    /// Error path: provider defined in models.json but no credential file →
    /// empty chain, no error and no panic (startup not blocked).
    #[tokio::test]
    async fn init_llm_registry_missing_credential_returns_empty_chain() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "models": [{ "id": "gpt-4o-basic", "enabled": true }]
                }
            }),
        )
        .unwrap();
        // Credentials dir intentionally absent.
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }

    /// Boundary: models.json defines no providers (placeholder content) →
    /// empty chain, startup not blocked.
    #[tokio::test]
    async fn init_llm_registry_empty_providers_returns_empty_chain() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }

    /// Boundary: credential_path pointing at an existing-but-unparseable
    /// file → the credential is swallowed by ConfigManager (non-strict load)
    /// → provider has no credential → skipped, empty chain, startup not
    /// blocked.
    ///
    /// NOTE: the models validator checks `credentialPath` existence
    /// CWD-relative at `ConfigManager::load`, so the path must be absolute
    /// for the config to load at all; `config_dir.join(abs_path)` resolves
    /// to the file itself.
    #[tokio::test]
    async fn init_llm_registry_broken_credential_path_returns_empty_chain() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        let broken = dir.path().join("broken.json");
        std::fs::write(&broken, "not json").unwrap();
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "credentialPath": broken.to_str().unwrap(),
                    "models": [{ "id": "gpt-4o-basic", "enabled": true }]
                }
            }),
        )
        .unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }

    /// Priority: credential_path wins over the convention directory for the
    /// same provider (ConfigManager merge rule). Absolute credentialPath
    /// (see note above) so the config loads despite the CWD-relative
    /// validator check.
    #[tokio::test]
    async fn init_llm_registry_credential_path_overrides_convention_dir() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        let explicit = dir.path().join("explicit.json");
        std::fs::write(&explicit, r#"{"provider":"openai","apiKey":"path-key"}"#).unwrap();
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "credentialPath": explicit.to_str().unwrap(),
                    "models": [{ "id": "gpt-4o-basic", "enabled": true }]
                }
            }),
        )
        .unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "openai", "dir-key").unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, _fallback_client) = init_registry_isolated(&cm).await;

        let provider = registry.get("openai").await.expect("openai registered");
        assert_eq!(provider.api_key(), "path-key");
    }

    /// The chain covers every enabled model across registered providers;
    /// models explicitly disabled are excluded; order is deterministic
    /// (sorted provider ids, declaration order within a provider).
    #[tokio::test]
    async fn init_llm_registry_chain_covers_enabled_models_only() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "models": [
                        { "id": "gpt-4o-basic", "enabled": true },
                        { "id": "gpt-4o-off", "enabled": false }
                    ]
                },
                "minimax": {
                    "models": [{ "id": "MiniMax-M2.7" }]
                }
            }),
        )
        .unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "openai", "k1").unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "minimax", "k2").unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert_eq!(registry.list().await.len(), 2);
        let chain = fallback_client.chain();
        assert_eq!(
            chain.len(),
            2,
            "disabled model excluded, absent flag included"
        );
        let model_ids: Vec<&str> = chain.iter().map(|entry| entry.model_id.as_str()).collect();
        assert_eq!(model_ids, vec!["MiniMax-M2.7", "gpt-4o-basic"]);
    }

    /// Vendor table lives in the llm crate: every provider implemented by
    /// closeclaw-llm — glm / deepseek / volcengine included, not just the
    /// original daemon-side four — is constructed from models.json +
    /// credentials instead of being silently skipped.
    #[tokio::test]
    async fn init_llm_registry_registers_all_llm_crate_vendors() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "glm": {
                    "baseUrl": "http://127.0.0.1:9/glm",
                    "models": [{ "id": "glm-4-plus" }]
                },
                "deepseek": {
                    "baseUrl": "http://127.0.0.1:9/deepseek",
                    "models": [{ "id": "deepseek-chat" }]
                },
                "volcengine": {
                    "baseUrl": "http://127.0.0.1:9/volc",
                    "models": [{ "id": "doubao-pro" }]
                }
            }),
        )
        .unwrap();
        for id in ["glm", "deepseek", "volcengine"] {
            crate::test_helpers::write_provider_credential(dir.path(), id, "k").unwrap();
        }
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert_eq!(registry.list().await.len(), 3, "all vendors registered");
        let glm = registry.get("glm").await.expect("glm registered");
        assert_eq!(glm.base_url(), "http://127.0.0.1:9/glm");
        let chain = fallback_client.chain();
        assert_eq!(chain.len(), 3);
        let model_ids: Vec<&str> = chain.iter().map(|e| e.model_id.as_str()).collect();
        // Sorted provider ids (deepseek < glm < volcengine), declaration
        // order within each provider.
        assert_eq!(model_ids, vec!["deepseek-chat", "glm-4-plus", "doubao-pro"]);
    }

    /// Vendor-default endpoint is used when models.json sets no baseUrl
    /// for a vendor added by this change (glm), matching the openai path.
    #[tokio::test]
    async fn init_llm_registry_vendor_default_base_url_without_config() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "glm": { "models": [{ "id": "glm-4-plus" }] }
            }),
        )
        .unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "glm", "k").unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, _fallback_client) = init_registry_isolated(&cm).await;

        let glm = registry.get("glm").await.expect("glm registered");
        assert_eq!(
            glm.base_url(),
            "https://open.bigmodel.cn/api/coding/paas/v4/chat/completions"
        );
    }

    /// A provider id without a known vendor implementation is skipped even
    /// when a credential exists (warn-logged, startup not blocked).
    #[tokio::test]
    async fn init_llm_registry_unknown_provider_skipped() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "acme": {
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "models": [{ "id": "acme-1", "enabled": true }]
                }
            }),
        )
        .unwrap();
        crate::test_helpers::write_provider_credential(dir.path(), "acme", "k").unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) = init_registry_isolated(&cm).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }

    /// Env fallback via the injected lookup: a provider without a
    /// credentials file still registers when `<PROVIDER>_API_KEY` resolves
    /// through the injected env lookup — proving the fallback path works
    /// without touching the real process environment.
    #[tokio::test]
    async fn init_llm_registry_env_fallback_via_injected_lookup() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": {
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "models": [{ "id": "gpt-4o-basic" }]
                }
            }),
        )
        .unwrap();
        // No credential file — the env lookup is the only key source.
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) =
            init_registry_with_env(&cm, &[("OPENAI_API_KEY", "sk-env")]).await;

        let provider = registry.get("openai").await.expect("openai registered");
        assert_eq!(provider.api_key(), "sk-env");
        assert_eq!(fallback_client.chain().len(), 1);
    }

    /// Empty env value is treated as absent (same rule as credential
    /// files) — injected lookup, deterministic.
    #[tokio::test]
    async fn init_llm_registry_empty_env_value_treated_as_absent() {
        let dir = TempDir::new().unwrap();
        write_config_skeleton(dir.path());
        crate::test_helpers::write_models_providers(
            dir.path(),
            serde_json::json!({
                "openai": { "models": [{ "id": "gpt-4o-basic" }] }
            }),
        )
        .unwrap();
        let cm = crate::test_helpers::load_config_manager(dir.path());

        let (registry, fallback_client) =
            init_registry_with_env(&cm, &[("OPENAI_API_KEY", "")]).await;

        assert!(registry.list().await.is_empty());
        assert_eq!(fallback_client.chain().len(), 0);
    }
}
