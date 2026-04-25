//! Models JSON ConfigProvider
//!
//! Loads and validates config/models.json configuration.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Model definition — id is required; name and enabled are optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefinition {
    pub id: String,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Single provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    #[serde(default)]
    pub base_url: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(rename = "api", default)]
    pub api: Option<String>,

    #[serde(default)]
    pub models: Vec<ModelDefinition>,
}

/// Root models configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelsConfigData {
    #[serde(default = "default_mode")]
    pub mode: String,

    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
}

fn default_mode() -> String {
    "merge".to_string()
}

impl Default for ModelsConfigData {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            providers: HashMap::new(),
        }
    }
}

impl ModelsConfigData {
    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: ModelsConfigData = serde_json::from_str(content)?;
        Ok(config)
    }

    /// Get a provider by id, if it exists.
    pub fn get_provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.get(id)
    }

    /// Get a model definition by provider id and model id.
    pub fn get_model(&self, provider_id: &str, model_id: &str) -> Option<&ModelDefinition> {
        self.providers
            .get(provider_id)
            .and_then(|p| p.models.iter().find(|m| m.id == model_id))
    }

    /// Return providers that have at least one enabled model.
    pub fn enabled_providers(&self) -> Vec<&str> {
        self.providers
            .iter()
            .filter(|(_, p)| p.models.iter().any(|m| m.enabled.unwrap_or(false)))
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

impl ConfigProvider for ModelsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (provider_id, provider) in &self.providers {
            if provider_id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: "providers".to_string(),
                    message: "provider id cannot be empty".to_string(),
                });
            }

            if let Some(ref base_url) = provider.base_url {
                if !base_url.is_empty()
                    && !(base_url.starts_with("http://") || base_url.starts_with("https://"))
                {
                    return Err(ConfigError::ValueError {
                        field: "base_url".to_string(),
                        message: "base_url must start with http:// or https://".to_string(),
                    });
                }
            }

            if let Some(ref api_key) = provider.api_key {
                if api_key.is_empty() {
                    return Err(ConfigError::ValueError {
                        field: "api_key".to_string(),
                        message: "api_key cannot be an empty string".to_string(),
                    });
                }
            }

            for model in &provider.models {
                if model.id.is_empty() {
                    return Err(ConfigError::ValueError {
                        field: "model.id".to_string(),
                        message: "model id cannot be empty".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "models.json"
    }

    fn is_default(&self) -> bool {
        self.mode == default_mode() && self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn default_config() -> ModelsConfigData {
        ModelsConfigData::default()
    }

    // -------------------------------------------------------------------------
    // Default config tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_config_is_valid() {
        let config = default_config();
        config.validate().expect("default config should be valid");
    }

    #[test]
    fn test_default_config_is_default() {
        let config = default_config();
        assert!(
            config.is_default(),
            "default config should return true from is_default()"
        );
    }

    // -------------------------------------------------------------------------
    // validate tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_empty_provider_id() {
        let json = r#"{
            "mode": "merge",
            "providers": {
                "": { "models": [] }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(result.is_err(), "empty provider id should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field == "providers"),
            "error should be about providers field"
        );
    }

    #[test]
    fn test_validate_empty_model_id() {
        let json = r#"{
            "mode": "merge",
            "providers": {
                "my-provider": {
                    "models": [{ "id": "" }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(result.is_err(), "empty model id should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field == "model.id"),
            "error should be about model.id field"
        );
    }

    #[test]
    fn test_validate_invalid_base_url() {
        let json = r#"{
            "mode": "merge",
            "providers": {
                "my-provider": {
                    "baseUrl": "ftp://example.com",
                    "models": [{ "id": "model-1" }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(result.is_err(), "non-http base_url should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field == "base_url"),
            "error should be about base_url field"
        );
    }

    #[test]
    fn test_validate_empty_api_key() {
        let json = r#"{
            "mode": "merge",
            "providers": {
                "my-provider": {
                    "apiKey": "",
                    "models": [{ "id": "model-1" }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(result.is_err(), "empty api_key should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field == "api_key"),
            "error should be about api_key field"
        );
    }

    #[test]
    fn test_validate_valid_config() {
        let json = r#"{
            "mode": "merge",
            "providers": {
                "my-provider": {
                    "baseUrl": "https://api.example.com",
                    "apiKey": "secret-key",
                    "models": [{ "id": "model-1", "name": "My Model", "enabled": true }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        config.validate().expect("valid config should pass");
    }

    // -------------------------------------------------------------------------
    // from_json_str tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_json_str_full() {
        let json = r#"{
            "mode": "replace",
            "providers": {
                "openai": {
                    "baseUrl": "https://api.openai.com",
                    "apiKey": "sk-test",
                    "api": "v1",
                    "models": [
                        { "id": "gpt-4", "name": "GPT-4", "enabled": true },
                        { "id": "gpt-3.5", "enabled": false }
                    ]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).expect("valid JSON should parse");
        assert_eq!(config.mode, "replace");
        assert_eq!(config.providers.len(), 1);
        let provider = config.providers.get("openai").unwrap();
        assert_eq!(provider.base_url.as_deref(), Some("https://api.openai.com"));
        assert_eq!(provider.api_key.as_deref(), Some("sk-test"));
        assert_eq!(provider.api.as_deref(), Some("v1"));
        assert_eq!(provider.models.len(), 2);
        assert_eq!(provider.models[0].id, "gpt-4");
        assert_eq!(provider.models[0].name.as_deref(), Some("GPT-4"));
        assert_eq!(provider.models[0].enabled, Some(true));
        assert_eq!(provider.models[1].id, "gpt-3.5");
        assert_eq!(provider.models[1].enabled, Some(false));
    }

    #[test]
    fn test_from_json_str_missing_optional_fields() {
        let json = r#"{
            "mode": "merge"
        }"#;
        let config = ModelsConfigData::from_json_str(json).expect("minimal JSON should parse");
        assert_eq!(config.mode, "merge");
        assert!(config.providers.is_empty());
    }

    #[test]
    fn test_from_json_str_invalid_json() {
        let result = ModelsConfigData::from_json_str("not json at all");
        assert!(result.is_err(), "invalid JSON should return Err");
    }

    #[test]
    fn test_from_json_str_empty_string() {
        let result = ModelsConfigData::from_json_str("");
        assert!(result.is_err(), "empty string should return Err");
    }

    // -------------------------------------------------------------------------
    // query interface tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_provider_hit() {
        let json = r#"{
            "providers": {
                "my-provider": { "models": [] }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        assert!(config.get_provider("my-provider").is_some());
    }

    #[test]
    fn test_get_provider_miss() {
        let config = default_config();
        assert!(config.get_provider("nonexistent").is_none());
    }

    #[test]
    fn test_get_model_hit() {
        let json = r#"{
            "providers": {
                "p1": {
                    "models": [{ "id": "m1", "name": "Model 1" }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let model = config.get_model("p1", "m1");
        assert!(model.is_some());
        assert_eq!(model.unwrap().name.as_deref(), Some("Model 1"));
    }

    #[test]
    fn test_get_model_miss() {
        let json = r#"{
            "providers": {
                "p1": { "models": [{ "id": "m1" }] }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        assert!(config.get_model("p1", "nonexistent").is_none());
        assert!(config.get_model("nonexistent", "m1").is_none());
    }

    #[test]
    fn test_enabled_providers() {
        let json = r#"{
            "providers": {
                "p1": {
                    "models": [{ "id": "m1", "enabled": true }, { "id": "m2", "enabled": false }]
                },
                "p2": {
                    "models": [{ "id": "m3", "enabled": false }]
                },
                "p3": {
                    "models": [{ "id": "m4", "enabled": true }]
                }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        let enabled = config.enabled_providers();
        assert!(enabled.contains(&"p1"));
        assert!(!enabled.contains(&"p2"));
        assert!(enabled.contains(&"p3"));
    }

    // -------------------------------------------------------------------------
    // is_default tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_default_mode_changed() {
        let json = r#"{
            "mode": "replace",
            "providers": {}
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        assert!(!config.is_default(), "mode != merge should not be default");
    }

    #[test]
    fn test_is_default_providers_not_empty() {
        let json = r#"{
            "providers": {
                "p1": { "models": [] }
            }
        }"#;
        let config = ModelsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "non-empty providers should not be default"
        );
    }

    // -------------------------------------------------------------------------
    // config_path and version
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_path() {
        assert_eq!(ModelsConfigData::config_path(), "models.json");
    }

    #[test]
    fn test_version() {
        let config = default_config();
        assert_eq!(config.version(), "1.0.0");
    }
}
