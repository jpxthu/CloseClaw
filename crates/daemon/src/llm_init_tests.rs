//! Unit tests for [`crate::llm_init`] — models.json-driven LLM registry.
//!
//! Behavior under test: the registry/chain content is determined solely by
//! models.json + credentials (convention directory, `credentialPath`,
//! injected env fallback) — never by the host process environment.

use super::*;
use std::path::Path;
use tempfile::TempDir;

/// Write the config skeleton into `dir` — the 5 mandatory files plus a
/// placeholder models.json (overwritten by `write_models_providers`
/// afterwards).
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
async fn test_init_llm_registry_registers_provider_with_configured_base_url() {
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
async fn test_init_llm_registry_missing_credential_returns_empty_chain() {
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
async fn test_init_llm_registry_empty_providers_returns_empty_chain() {
    let dir = TempDir::new().unwrap();
    write_config_skeleton(dir.path());
    let cm = crate::test_helpers::load_config_manager(dir.path());

    let (registry, fallback_client) = init_registry_isolated(&cm).await;

    assert!(registry.list().await.is_empty());
    assert_eq!(fallback_client.chain().len(), 0);
}

/// Boundary: models.json absent (every other mandatory config present)
/// → `ConfigManager::load` treats it as an optional section (INFO, no
/// error — models.json no longer gates startup) → the Models section
/// stays `None` → `init_llm_registry` reads the INFO branch of
/// `ConfigManager::models_config`: empty chain, no error, no panic.
/// Design doc `docs/design/daemon/README.md`: 「models.json 缺失…系统仍
/// 正常启动」. The full-startup counterpart is pinned by
/// `tests.rs::test_daemon_start_succeeds_without_models_json`.
#[tokio::test]
async fn test_init_llm_registry_missing_models_json_returns_empty_chain() {
    let dir = TempDir::new().unwrap();
    // Every mandatory config except models.json — deliberately not written.
    closeclaw_common::test_helpers::write_mandatory_without_models(dir.path())
        .expect("mandatory configs without models.json");
    let cm = ConfigManager::new(dir.path().to_path_buf()).unwrap();
    cm.load()
        .expect("models.json is optional — load must succeed");
    assert!(cm.section(ConfigSection::Models).is_none());

    let (registry, fallback_client) = init_registry_isolated(&cm).await;

    assert!(registry.list().await.is_empty());
    assert_eq!(fallback_client.chain().len(), 0);
}

/// Boundary: models.json present but corrupt (invalid JSON) with no
/// backup → F3 protection (config README 启动加载 step 1 + requirements
/// config §F3): `ConfigManager::load` refuses startup — the daemon never
/// reaches `init_llm_registry` (empty chain is not even constructed).
#[tokio::test]
async fn test_init_llm_registry_corrupt_models_json_refuses_startup() {
    let dir = TempDir::new().unwrap();
    closeclaw_common::test_helpers::write_mandatory_without_models(dir.path())
        .expect("mandatory configs without models.json");
    std::fs::write(dir.path().join("models.json"), "not valid json {{").unwrap();
    let cm = ConfigManager::new(dir.path().to_path_buf()).unwrap();

    let err = cm
        .load()
        .expect_err("corrupt models.json without backup must refuse startup");
    assert!(err.to_string().contains("models.json"), "{err}");
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
async fn test_init_llm_registry_broken_credential_path_returns_empty_chain() {
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
async fn test_init_llm_registry_credential_path_overrides_convention_dir() {
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
async fn test_init_llm_registry_chain_covers_enabled_models_only() {
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
async fn test_init_llm_registry_registers_all_llm_crate_vendors() {
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
async fn test_init_llm_registry_vendor_default_base_url_without_config() {
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
async fn test_init_llm_registry_unknown_provider_skipped() {
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
async fn test_init_llm_registry_env_fallback_via_injected_lookup() {
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
async fn test_init_llm_registry_empty_env_value_treated_as_absent() {
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

    let (registry, fallback_client) = init_registry_with_env(&cm, &[("OPENAI_API_KEY", "")]).await;

    assert!(registry.list().await.is_empty());
    assert_eq!(fallback_client.chain().len(), 0);
}
