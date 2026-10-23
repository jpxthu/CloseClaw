//! Config file writing for the config wizard.

use closeclaw_config::providers::credentials::{AnyProviderCredentials, ApiKeyCredentials};
use closeclaw_config::providers::models::{ModelDefinition, ModelsConfigData, ProviderConfig};
use closeclaw_llm::model_info::ModelInfo;
use closeclaw_llm::ProviderModelKnowledge;

use std::path::{Path, PathBuf};

use super::types::WizardOutput;

/// Locate the project config directory (`~/.closeclaw/config/`).
fn config_dir() -> anyhow::Result<PathBuf> {
    let mgr =
        closeclaw_config::ConfigManager::with_default_root_dir().map_err(|e| anyhow::anyhow!(e))?;
    Ok(mgr.config_dir().to_path_buf())
}

/// Merge selected models with existing models for a provider.
///
/// Strategy:
/// - Existing models not in `selected_models` are preserved (not deleted)
/// - Existing models matching by id are replaced entirely (name/enabled updated)
/// - New models from `selected_models` are appended
fn merge_models(
    existing_models: Vec<ModelDefinition>,
    selected_models: &[ModelInfo],
) -> Vec<ModelDefinition> {
    let mut merged: Vec<ModelDefinition> = existing_models;
    for selected in selected_models {
        if let Some(existing) = merged.iter_mut().find(|m| m.id == selected.id) {
            *existing = ModelDefinition {
                id: selected.id.clone(),
                name: Some(selected.name.clone()),
                enabled: Some(true),
            };
        } else {
            merged.push(ModelDefinition {
                id: selected.id.clone(),
                name: Some(selected.name.clone()),
                enabled: Some(true),
            });
        }
    }
    merged
}

/// Inherit base_url, api_key, api from an existing provider config if present.
fn inherit_provider_fields(
    existing_provider: &Option<ProviderConfig>,
) -> (Option<String>, Option<String>, Option<String>) {
    existing_provider
        .as_ref()
        .map(|ep| (ep.base_url.clone(), ep.api_key.clone(), ep.api.clone()))
        .unwrap_or((None, None, None))
}

/// Load existing models config or return a default.
fn load_or_create_models_config(models_path: &Path) -> anyhow::Result<ModelsConfigData> {
    if models_path.exists() {
        let content = std::fs::read_to_string(models_path)?;
        Ok(ModelsConfigData::from_json_str(&content)
            .unwrap_or_else(|_| ModelsConfigData::default()))
    } else {
        Ok(ModelsConfigData::default())
    }
}

/// Write provider credentials to `credentials/<provider_id>.json`.
fn write_provider_credentials(
    config_path: &Path,
    provider_id: &str,
    credential: &str,
) -> anyhow::Result<PathBuf> {
    let creds_dir = config_path.join("credentials");
    std::fs::create_dir_all(&creds_dir)?;
    let cred_file = creds_dir.join(format!("{}.json", provider_id));
    let creds = AnyProviderCredentials::ApiKey(ApiKeyCredentials {
        provider: provider_id.to_string(),
        api_key: credential.to_string(),
    });
    let creds_json = serde_json::to_string_pretty(&creds)?;
    std::fs::write(&cred_file, creds_json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", cred_file.display(), e))?;
    Ok(cred_file)
}

/// Write wizard output to config files.
///
/// Merging strategy:
/// - New selected models for the chosen provider: add/update with their parameters
/// - Already-configured models for other providers: preserved
/// - Already-configured models for this provider that were NOT selected: preserved
///   (so running wizard multiple times does not lose manual overrides)
///
/// Files written:
/// - `~/.closeclaw/config/models.json`   — merged provider+model config
/// - `~/.closeclaw/config/credentials/<provider_id>.json` — API key for the provider
//-
//- NOTE: Any change to `write_wizard_config` or `run_wizard` must be verified
//- against `tests/cli_config_wizard_test.py` (python3 E2E test using pexpect).
pub fn write_wizard_config_to(output: &WizardOutput, config_path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_path)?;
    let models_path = config_path.join("models.json");
    let existing = load_or_create_models_config(&models_path)?;
    let mut providers = existing.providers;
    let existing_provider = providers.get(&output.provider_id).cloned();
    providers.remove(&output.provider_id);
    let existing_models = existing_provider
        .as_ref()
        .map(|ep| ep.models.clone())
        .unwrap_or_default();
    let merged_models = merge_models(existing_models, &output.selected_models);
    let recommended_protocol = output.selected_models.first().map(|m| {
        let kb = ProviderModelKnowledge::new();
        kb.recommended_protocol(&output.provider_id, &m.id)
            .to_string()
    });
    let (base_url, _api_key, api) = inherit_provider_fields(&existing_provider);
    // api_key is never stored in models.json — credentials live in a separate
    // file referenced by credential_path. Setting it to None prevents leaking
    // secrets into the config file.
    providers.insert(
        output.provider_id.clone(),
        ProviderConfig {
            base_url,
            api_key: None,
            api,
            protocol: recommended_protocol,
            credential_path: Some(format!("credentials/{}.json", output.provider_id)),
            models: merged_models,
        },
    );
    let merged = ModelsConfigData {
        mode: "merge".to_string(),
        providers,
    };
    <ModelsConfigData as closeclaw_config::providers::ConfigProvider>::validate(&merged)
        .map_err(|e| anyhow::anyhow!("merged config validation failed: {}", e))?;
    let json = serde_json::to_string_pretty(&merged)?;
    std::fs::write(&models_path, json)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", models_path.display(), e))?;
    let cred_file =
        write_provider_credentials(config_path, &output.provider_id, &output.credential)?;
    println!(
        "✅ Configuration written:\n   models: {}\n   credentials: {}",
        models_path.display(),
        cred_file.display()
    );
    Ok(())
}

/// Write wizard output to config files in the default config directory.
pub fn write_wizard_config(output: &WizardOutput) -> anyhow::Result<()> {
    write_wizard_config_to(output, &config_dir()?)
}

/// Locate the models.json config file, if it exists.
pub(super) fn find_models_config() -> Option<std::path::PathBuf> {
    let possible = ["config/models.json", "configs/models.json"];
    possible
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}
