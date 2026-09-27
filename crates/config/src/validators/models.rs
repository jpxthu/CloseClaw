//! Models config section validator.

use super::cross_ref::CredentialProviderSet;
use super::helpers::{ensure_array, ensure_object};

/// Validate the **models** config section (basic structural checks).
pub fn validate_models(value: &serde_json::Value) -> Result<(), String> {
    validate_models_with_refs(value, None)
}

/// Validate the **models** config section with optional credential cross-ref.
pub fn validate_models_with_refs(
    value: &serde_json::Value,
    credential_providers: Option<&CredentialProviderSet>,
) -> Result<(), String> {
    ensure_object(value, "models")?;
    if let Some(arr) = value.get("models") {
        ensure_array(arr, "models.models")?;
    }
    // Business validation: iterate providers and models
    if let Some(providers) = value.get("providers") {
        if let Some(obj) = providers.as_object() {
            for (provider_id, provider_val) in obj {
                if provider_id.is_empty() {
                    return Err("models provider ID cannot be empty".to_string());
                }
                validate_provider(provider_id, provider_val, credential_providers)?;
            }
        }
    }
    Ok(())
}

/// Validate a single provider entry within the models section.
fn validate_provider(
    provider_id: &str,
    provider: &serde_json::Value,
    credential_providers: Option<&CredentialProviderSet>,
) -> Result<(), String> {
    if !provider.is_object() {
        return Err(format!(
            "models.providers.{} must be a JSON object",
            provider_id
        ));
    }
    // Validate base_url format if present
    if let Some(base_url) = provider.get("baseUrl") {
        if let Some(url) = base_url.as_str() {
            if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(format!(
                    "models.providers.{}.baseUrl must start with \
                     http:// or https://",
                    provider_id
                ));
            }
        }
    }
    // Validate credentialPath format and existence if present
    if let Some(cred_path) = provider.get("credentialPath") {
        if let Some(path_str) = cred_path.as_str() {
            if path_str.is_empty() {
                return Err(format!(
                    "models.providers.{}.credentialPath cannot be empty",
                    provider_id
                ));
            }
            if !std::path::Path::new(path_str).exists() {
                return Err(format!(
                    "models.providers.{}.credentialPath '{}' does not exist",
                    provider_id, path_str
                ));
            }
        } else if !cred_path.is_null() {
            return Err(format!(
                "models.providers.{}.credentialPath must be a string",
                provider_id
            ));
        }
    }
    // Cross-validate apiKey: provider must exist in credentials if apiKey is set
    if let Some(api_key) = provider.get("apiKey") {
        if api_key.is_string() {
            if let Some(crps) = credential_providers {
                let has_credential_path = provider
                    .get("credentialPath")
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                if !has_credential_path && !crps.names.contains(provider_id) {
                    return Err(format!(
                        "models.providers.{}.apiKey references an unknown credential provider. \
                         Known providers: {}",
                        provider_id,
                        crps.names
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
    }
    // Validate each model entry
    if let Some(models) = provider.get("models") {
        if let Some(arr) = models.as_array() {
            for model in arr {
                validate_model(provider_id, model)?;
            }
        }
    }
    Ok(())
}

/// Validate a single model entry within a provider.
fn validate_model(provider_id: &str, model: &serde_json::Value) -> Result<(), String> {
    if !model.is_object() {
        return Err(format!(
            "models.providers.{}.models[] must be objects",
            provider_id
        ));
    }
    // Model ID is required and must be non-empty
    match model.get("id") {
        Some(serde_json::Value::String(id)) if id.is_empty() => {
            return Err(format!(
                "models.providers.{}.models[].id cannot be empty",
                provider_id
            ));
        }
        None => {
            return Err(format!(
                "models.providers.{}.models[].id is required",
                provider_id
            ));
        }
        _ => {}
    }
    Ok(())
}
