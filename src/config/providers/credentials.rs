//! Credentials JSON ConfigProvider
//!
//! Loads and validates per-provider credential files from config/credentials/.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// API key credentials for a generic provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyCredentials {
    pub provider: String,
    pub api_key: String,
}

/// Feishu-specific credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeishuCredentials {
    pub provider: String,
    pub app_id: String,
    pub app_secret: String,

    #[serde(default)]
    pub bot_name: Option<String>,
}

/// Untagged credentials supporting multiple provider shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AnyProviderCredentials {
    ApiKey(ApiKeyCredentials),
    Feishu(FeishuCredentials),
}

/// Root credentials provider — holds all loaded credentials by provider name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CredentialsProvider {
    #[serde(default)]
    pub providers: HashMap<String, AnyProviderCredentials>,
}

impl Default for CredentialsProvider {
    fn default() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }
}

impl CredentialsProvider {
    /// Load all credentials from a directory containing JSON files.
    ///
    /// Each file should contain a single credentials object.
    /// Returns an empty provider if the directory does not exist.
    pub fn load_from_dir(dir: &Path) -> Result<Self, ConfigError> {
        if !dir.exists() {
            return Ok(Self::default());
        }

        let mut provider = CredentialsProvider::default();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            let creds: AnyProviderCredentials = match serde_json::from_str(&content) {
                Ok(c) => c,
                Err(_) => {
                    // skip malformed files silently
                    continue;
                }
            };
            let name = match &creds {
                AnyProviderCredentials::ApiKey(c) => c.provider.clone(),
                AnyProviderCredentials::Feishu(c) => c.provider.clone(),
            };
            provider.providers.insert(name, creds);
        }
        Ok(provider)
    }

    /// Parse from a JSON string (useful for tests).
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let provider: CredentialsProvider = serde_json::from_str(content)?;
        Ok(provider)
    }

    /// Get credentials for a named provider.
    pub fn get(&self, provider: &str) -> Option<&AnyProviderCredentials> {
        self.providers.get(provider)
    }

    /// Get the api_key for a named provider.
    ///
    /// Returns `None` if the provider does not exist or is not an ApiKey variant.
    pub fn get_api_key(&self, provider: &str) -> Option<String> {
        match self.providers.get(provider)? {
            AnyProviderCredentials::ApiKey(c) => Some(c.api_key.clone()),
            AnyProviderCredentials::Feishu(_) => None,
        }
    }

    /// Get Feishu credentials if a feishu provider exists.
    pub fn feishu_creds(&self) -> Option<&FeishuCredentials> {
        self.providers.values().find_map(|c| match c {
            AnyProviderCredentials::Feishu(f) => Some(f),
            AnyProviderCredentials::ApiKey(_) => None,
        })
    }
}

impl ConfigProvider for CredentialsProvider {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (name, creds) in &self.providers {
            match creds {
                AnyProviderCredentials::ApiKey(c) => {
                    if c.api_key.is_empty() {
                        return Err(ConfigError::ValueError {
                            field: format!("{}.api_key", name),
                            message: "api_key cannot be empty".to_string(),
                        });
                    }
                }
                AnyProviderCredentials::Feishu(f) => {
                    if f.app_id.is_empty() {
                        return Err(ConfigError::ValueError {
                            field: format!("{}.app_id", name),
                            message: "app_id cannot be empty".to_string(),
                        });
                    }
                    if f.app_secret.is_empty() {
                        return Err(ConfigError::ValueError {
                            field: format!("{}.app_secret", name),
                            message: "app_secret cannot be empty".to_string(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "credentials/"
    }

    fn is_default(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn default_provider() -> CredentialsProvider {
        CredentialsProvider::default()
    }

    // -------------------------------------------------------------------------
    // Default config tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_config_is_valid() {
        let provider = default_provider();
        provider.validate().expect("default should be valid");
    }

    #[test]
    fn test_default_config_is_default() {
        let provider = default_provider();
        assert!(provider.is_default());
    }

    // -------------------------------------------------------------------------
    // load_from_dir tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_load_from_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let provider = CredentialsProvider::load_from_dir(tmp.path()).unwrap();
        assert!(provider.is_default());
        assert!(provider.providers.is_empty());
    }

    #[test]
    fn test_load_from_nonexistent_dir() {
        let provider =
            CredentialsProvider::load_from_dir(Path::new("/nonexistent/path/that/does/not/exist"))
                .unwrap();
        assert!(provider.is_default());
    }

    #[test]
    fn test_load_api_key_credential() {
        let tmp = TempDir::new().unwrap();
        let content = r#"{"provider":"openai","apiKey":"sk-test123"}"#;
        fs::write(tmp.path().join("openai.json"), content).unwrap();
        let provider = CredentialsProvider::load_from_dir(tmp.path()).unwrap();
        assert_eq!(provider.providers.len(), 1);
        let api_key = provider.get_api_key("openai").unwrap();
        assert_eq!(api_key, "sk-test123");
    }

    #[test]
    fn test_load_feishu_credential() {
        let tmp = TempDir::new().unwrap();
        let content = r#"{
            "provider": "feishu",
            "appId": "cli_abc123",
            "appSecret": "secret_xyz"
        }"#;
        fs::write(tmp.path().join("feishu.json"), content).unwrap();
        let provider = CredentialsProvider::load_from_dir(tmp.path()).unwrap();
        assert_eq!(provider.providers.len(), 1);
        let feishu = provider.feishu_creds().unwrap();
        assert_eq!(feishu.app_id, "cli_abc123");
        assert_eq!(feishu.app_secret, "secret_xyz");
    }

    #[test]
    fn test_load_multiple_providers() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("openai.json"),
            r#"{"provider":"openai","apiKey":"sk-openai"}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("anthropic.json"),
            r#"{"provider":"anthropic","apiKey":"sk-ant"}"#,
        )
        .unwrap();
        let provider = CredentialsProvider::load_from_dir(tmp.path()).unwrap();
        assert_eq!(provider.providers.len(), 2);
        assert_eq!(provider.get_api_key("openai").unwrap(), "sk-openai");
        assert_eq!(provider.get_api_key("anthropic").unwrap(), "sk-ant");
    }

    // -------------------------------------------------------------------------
    // validate tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_empty_api_key() {
        let json = r#"{"providers":{
            "my-provider": {"provider":"my-provider","apiKey":""}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        let result = provider.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::ValueError { ref field, .. }
            if field.contains("api_key")));
    }

    #[test]
    fn test_validate_feishu_empty_app_id() {
        let json = r#"{"providers":{
            "feishu": {"provider":"feishu","appId":"","appSecret":"somesecret"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        let result = provider.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::ValueError { ref field, .. }
            if field.contains("app_id")));
    }

    #[test]
    fn test_validate_valid_api_key() {
        let json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-valid"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        provider.validate().expect("valid config should pass");
    }

    #[test]
    fn test_validate_valid_feishu() {
        let json = r#"{"providers":{
            "feishu":{"provider":"feishu","appId":"cli_abc","appSecret":"sec","botName":"Bot"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        provider
            .validate()
            .expect("valid feishu config should pass");
    }

    // -------------------------------------------------------------------------
    // query interface tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_api_key() {
        let json = r#"{"providers":{
            "p1": {"provider":"p1","apiKey":"key1"},
            "p2": {"provider":"p2","apiKey":"key2"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        assert_eq!(provider.get_api_key("p1").unwrap(), "key1");
        assert_eq!(provider.get_api_key("p2").unwrap(), "key2");
        assert!(provider.get_api_key("p3").is_none());
    }

    #[test]
    fn test_feishu_creds() {
        let json = r#"{"providers":{
            "feishu": {"provider":"feishu","appId":"id","appSecret":"secret","botName":"Bot"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        let feishu = provider.feishu_creds().unwrap();
        assert_eq!(feishu.app_id, "id");
        assert_eq!(feishu.bot_name.as_deref(), Some("Bot"));
    }

    #[test]
    fn test_feishu_creds_none_when_missing() {
        let json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        assert!(provider.feishu_creds().is_none());
    }

    // -------------------------------------------------------------------------
    // is_default tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_default_empty() {
        let provider = default_provider();
        assert!(provider.is_default());
    }

    #[test]
    fn test_is_default_not_empty() {
        let json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        assert!(!provider.is_default());
    }

    // -------------------------------------------------------------------------
    // config_path and version
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_path() {
        assert_eq!(CredentialsProvider::config_path(), "credentials/");
    }

    #[test]
    fn test_version() {
        let provider = default_provider();
        assert_eq!(provider.version(), "1.0.0");
    }
}
