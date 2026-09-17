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
use closeclaw_config::providers::CredentialsProvider;
use closeclaw_llm::call_chain;
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
    /// the system still starts normally when LLM capability is missing).
    ///
    /// `env_lookup` is the `<PROVIDER>_API_KEY` fallback: production
    /// passes [`process_env`] (`std::env::var`), tests inject a
    /// deterministic stub so outcomes never depend on host env vars.
    pub(crate) async fn init_llm_registry<F>(
        config_manager: &ConfigManager,
        env_lookup: F,
    ) -> (Arc<LLMRegistry>, Arc<UnifiedFallbackClient>)
    where
        F: Fn(&str) -> Option<String>,
    {
        let registry = Arc::new(LLMRegistry::new());
        // Single-point models.json access (config cache, filled at load):
        // absent → empty default. Present-file parse/validation failures
        // are refused earlier by `load()` (F3) and never reach here.
        let models = config_manager.models_config();
        // Credentials are resolved at ConfigManager::load: convention directory
        // + credential_path merge (credential_path wins on conflicts).
        let credentials = match config_manager.credentials() {
            Some(credentials) => credentials,
            None => {
                tracing::warn!("ConfigManager credentials not loaded — using empty set");
                CredentialsProvider::default()
            }
        };

        // Sorted provider ids → deterministic registry/chain order.
        let mut provider_ids: Vec<&String> = models.providers.keys().collect();
        provider_ids.sort();
        let mut chain_entries: Vec<ChainEntry> = Vec::new();
        for provider_id in provider_ids {
            let provider_cfg = &models.providers[provider_id];
            let Some(api_key) = resolve_api_key(&credentials, provider_id, &env_lookup) else {
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
}

/// Production env lookup for the api-key fallback: read-only
/// `std::env::var` (STANDARDS §7 — the forbidden write variants are
/// not used here). Passed into [`Daemon::init_llm_registry`] as the
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

/// Build fallback-chain entries for one registered provider.
///
/// One entry per enabled model, filtered by the single predicate
/// [`closeclaw_config::providers::models::ModelDefinition::is_enabled`]
/// (config crate) — `enabled` defaults to true when the flag is omitted,
/// so a model listed in models.json is usable unless explicitly disabled
/// (`enabled: false`). `model_id` carries the real model id from
/// models.json so the outbound request names the configured model.
/// Entry assembly is the llm-crate single point
/// [`closeclaw_llm::call_chain::build_chain_entry`] — daemon no longer
/// touches `UnifiedChatClient` / cache-adapter internals itself.
fn chain_entries_for(
    provider_id: &str,
    provider: &DynProvider,
    provider_cfg: &ProviderConfig,
) -> Vec<ChainEntry> {
    let mut entries = Vec::new();
    for model in &provider_cfg.models {
        if !model.is_enabled() {
            continue;
        }
        entries.push(call_chain::build_chain_entry(
            Arc::clone(provider),
            provider_id,
            &model.id,
        ));
    }
    entries
}

#[cfg(test)]
#[path = "llm_init_tests.rs"]
mod tests;
