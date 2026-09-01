//! Credentials JSON ConfigProvider
//!
//! Loads and validates per-provider credential files from config/credentials/.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::providers::{ConfigError, ModelsConfigData};
use crate::ConfigProvider;

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

/// Feishu profile — credentials are managed by lark-cli via profile name.
///
/// The adapter only needs the profile name; all credential management
/// (token refresh, secret storage) is handled by lark-cli.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FeishuProfile {
    pub provider: String,
    pub profile: String,

    #[serde(default)]
    pub bot_name: Option<String>,
}

/// Untagged credentials supporting multiple provider shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AnyProviderCredentials {
    ApiKey(ApiKeyCredentials),
    Feishu(FeishuProfile),
}

/// Root credentials provider — holds all loaded credentials by provider name.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CredentialsProvider {
    #[serde(default)]
    pub providers: HashMap<String, AnyProviderCredentials>,
}

impl CredentialsProvider {
    /// Load a single credential file.
    ///
    /// The file should contain a single credentials object.
    /// Returns an empty provider if the file does not exist.
    ///
    /// **Note:** Parse/validation failures are silently swallowed and
    /// return an empty provider. For hot-reload paths that must surface
    /// errors, use [`load_from_file_strict`](Self::load_from_file_strict).
    pub fn load_from_file(file: &Path) -> Result<Self, ConfigError> {
        let mut provider = CredentialsProvider::default();
        if !file.exists() {
            return Ok(provider);
        }
        let content = fs::read_to_string(file)?;
        let creds: AnyProviderCredentials = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(_) => {
                return Ok(provider);
            }
        };
        let name = match &creds {
            AnyProviderCredentials::ApiKey(c) => c.provider.clone(),
            AnyProviderCredentials::Feishu(c) => c.provider.clone(),
        };
        provider.providers.insert(name, creds);
        Ok(provider)
    }

    /// Strict variant of [`load_from_file`](Self::load_from_file) that
    /// returns `Err` on parse or validation failure.
    ///
    /// Used by the hot-reload path to ensure credential_path references
    /// with invalid files abort the entire load rather than being silently
    /// skipped.
    pub fn load_from_file_strict(file: &Path) -> Result<Self, ConfigError> {
        if !file.exists() {
            return Err(ConfigError::ParseError {
                path: file.to_path_buf(),
                error: "credential_path file does not exist".to_string(),
            });
        }
        let content = fs::read_to_string(file)?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        if let Err(e) = crate::validators::validate_credentials(&value) {
            return Err(ConfigError::ValidationError {
                path: file.to_path_buf(),
                message: e,
            });
        }
        let creds: AnyProviderCredentials = serde_json::from_value(value)?;
        let mut provider = CredentialsProvider::default();
        let name = match &creds {
            AnyProviderCredentials::ApiKey(c) => c.provider.clone(),
            AnyProviderCredentials::Feishu(c) => c.provider.clone(),
        };
        provider.providers.insert(name, creds);
        Ok(provider)
    }

    /// Load all credentials from a directory containing JSON files.
    ///
    /// Each file should contain a single credentials object.
    /// Returns an empty provider if the directory does not exist.
    ///
    /// When `strict` is `true`, the first parsing or validation failure
    /// aborts the entire load and returns an error. This is used by the
    /// hot-reload path so that validation failures are surfaced to the
    /// caller (which triggers `on_validation_failed` and prevents staging).
    pub fn load_from_dir(dir: &Path) -> Result<Self, ConfigError> {
        Self::load_from_dir_with_mode(dir, false)
    }

    /// Strict variant of [`load_from_dir`](Self::load_from_dir) that
    /// fails on the first invalid credential file.
    ///
    /// Used by the hot-reload path to ensure validation failures are
    /// not silently swallowed.
    pub fn load_from_dir_strict(dir: &Path) -> Result<Self, ConfigError> {
        Self::load_from_dir_with_mode(dir, true)
    }

    fn load_from_dir_with_mode(dir: &Path, strict: bool) -> Result<Self, ConfigError> {
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
            if let Some((name, creds)) = Self::parse_credential_file(&path, strict)? {
                ensure_owner_only_permissions(&path).unwrap_or_else(|e| {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to set credential file permissions"
                    );
                });
                provider.providers.insert(name, creds);
            }
        }
        Ok(provider)
    }

    /// Parse, validate, and deserialize a single credential file.
    ///
    /// Returns `Ok(Some((name, creds)))` on success, `Ok(None)` when the file
    /// should be skipped (non-strict mode parse/validation failure), or
    /// `Err` in strict mode when the first failure aborts the entire load.
    fn parse_credential_file(
        path: &Path,
        strict: bool,
    ) -> Result<Option<(String, AnyProviderCredentials)>, ConfigError> {
        let content = fs::read_to_string(path)?;
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                if strict {
                    return Err(ConfigError::ParseError {
                        path: path.to_path_buf(),
                        error: e.to_string(),
                    });
                }
                return Ok(None);
            }
        };
        // Validate credential file structure before deserializing
        if let Err(e) = crate::validators::validate_credentials(&value) {
            if strict {
                return Err(ConfigError::ValidationError {
                    path: path.to_path_buf(),
                    message: e,
                });
            }
            warn!(
                path = %path.display(),
                error = %e,
                "credential file failed validation, skipping"
            );
            return Ok(None);
        }
        let creds: AnyProviderCredentials = match serde_json::from_value(value) {
            Ok(c) => c,
            Err(_) => {
                if strict {
                    return Err(ConfigError::ParseError {
                        path: path.to_path_buf(),
                        error: "credential file does not match any known credential shape"
                            .to_string(),
                    });
                }
                return Ok(None);
            }
        };
        let name = match &creds {
            AnyProviderCredentials::ApiKey(c) => c.provider.clone(),
            AnyProviderCredentials::Feishu(c) => c.provider.clone(),
        };
        Ok(Some((name, creds)))
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

    /// Get Feishu profile if a feishu provider exists.
    pub fn feishu_profile(&self) -> Option<&FeishuProfile> {
        self.providers.values().find_map(|c| match c {
            AnyProviderCredentials::Feishu(f) => Some(f),
            AnyProviderCredentials::ApiKey(_) => None,
        })
    }

    /// Cross-validate that every provider referenced in models.json has
    /// corresponding credentials defined.
    ///
    /// Credential resolution priority:
    /// 1. In-memory credentials (loaded from convention directory or
    ///    credential_path during `load()`).
    /// 2. `credential_path` in models.json — if set and the target file
    ///    exists, the provider is considered valid.
    /// 3. Convention directory `config/credentials/<provider>.json`.
    ///
    /// Returns `Err` if any model provider has none of the above.
    /// Extra credentials (providers defined in credentials but not in models)
    /// emit a warning but do not fail validation.
    pub fn validate_model_references(
        &self,
        models_provider: &ModelsConfigData,
        config_dir: &Path,
    ) -> Result<(), ConfigError> {
        for provider_id in models_provider.providers.keys() {
            if self.providers.contains_key(provider_id) {
                continue;
            }

            // Check if the provider has a credential_path in models.json.
            if let Some(provider_cfg) = models_provider.get_provider(provider_id) {
                if let Some(ref rel_path) = provider_cfg.credential_path {
                    if !rel_path.is_empty() {
                        let abs_path = config_dir.join(rel_path);
                        if abs_path.exists() {
                            info!(
                                provider = %provider_id,
                                path = %abs_path.display(),
                                "provider '{}' has valid credential_path", provider_id
                            );
                            continue;
                        }
                    }
                }
            }

            return Err(ConfigError::ValueError {
                field: format!("credentials.{}", provider_id),
                message: format!(
                    "provider '{}' is referenced in models.json but has no credentials \
                     (set credentialPath in models.json, create a credentials file in \
                     config/credentials/, or check the provider ID)",
                    provider_id
                ),
            });
        }

        for cred_id in self.providers.keys() {
            if !models_provider.providers.contains_key(cred_id) {
                warn!(
                    provider = %cred_id,
                    "credentials defined for provider '{}' but not referenced in models.json",
                    cred_id
                );
            }
        }

        Ok(())
    }
}

/// Ensure the credential file has owner-only permissions (0o600).
///
/// Returns Ok(()) if the permissions are already correct or were successfully
/// updated. Returns Err with the OS error if the permission check or update
/// fails.
fn ensure_owner_only_permissions(path: &Path) -> Result<(), std::io::Error> {
    const OWNER_ONLY: u32 = 0o600;

    let metadata = fs::metadata(path)?;
    let current_mode = metadata.permissions().mode() & 0o777;

    if current_mode == OWNER_ONLY {
        return Ok(());
    }

    fs::set_permissions(path, fs::Permissions::from_mode(OWNER_ONLY))
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
                    if f.profile.is_empty() {
                        return Err(ConfigError::ValueError {
                            field: format!("{}.profile", name),
                            message: "profile cannot be empty".to_string(),
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
            "profile": "my_feishu_profile"
        }"#;
        fs::write(tmp.path().join("feishu.json"), content).unwrap();
        let provider = CredentialsProvider::load_from_dir(tmp.path()).unwrap();
        assert_eq!(provider.providers.len(), 1);
        let feishu = provider.feishu_profile().unwrap();
        assert_eq!(feishu.profile, "my_feishu_profile");
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
    // load_from_file_strict tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_load_from_file_strict_success() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("openai.json");
        fs::write(&file, r#"{"provider":"openai","apiKey":"sk-test"}"#).unwrap();
        let provider = CredentialsProvider::load_from_file_strict(&file).unwrap();
        assert_eq!(provider.providers.len(), 1);
        assert_eq!(provider.get_api_key("openai").unwrap(), "sk-test");
    }

    #[test]
    fn test_load_from_file_strict_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("nonexistent.json");
        let result = CredentialsProvider::load_from_file_strict(&file);
        assert!(result.is_err(), "should fail for nonexistent file");
    }

    #[test]
    fn test_load_from_file_strict_parse_error() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("bad.json");
        fs::write(&file, r#"{not valid json"#).unwrap();
        let result = CredentialsProvider::load_from_file_strict(&file);
        assert!(result.is_err(), "should fail for malformed JSON");
    }

    #[test]
    fn test_load_from_file_strict_validation_error() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("invalid.json");
        fs::write(&file, r#"{"provider":"openai","apiKey":""}"#).unwrap();
        let result = CredentialsProvider::load_from_file_strict(&file);
        assert!(result.is_err(), "should fail for validation error");
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
    fn test_validate_feishu_empty_profile() {
        let json = r#"{"providers":{
            "feishu": {"provider":"feishu","profile":""}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        let result = provider.validate();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::ValueError { ref field, .. }
            if field.contains("profile")));
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
            "feishu":{"provider":"feishu","profile":"my_profile","botName":"Bot"}
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
    fn test_feishu_profile() {
        let json = r#"{"providers":{
            "feishu": {"provider":"feishu","profile":"my_profile","botName":"Bot"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        let feishu = provider.feishu_profile().unwrap();
        assert_eq!(feishu.profile, "my_profile");
        assert_eq!(feishu.bot_name.as_deref(), Some("Bot"));
    }

    #[test]
    fn test_feishu_profile_none_when_missing() {
        let json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"}
        }}"#;
        let provider = CredentialsProvider::from_json_str(json).unwrap();
        assert!(provider.feishu_profile().is_none());
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

    // -----------------------------------------------------------------
    // validate_model_references tests
    // -----------------------------------------------------------------

    fn models_from_json(json: &str) -> super::ModelsConfigData {
        super::ModelsConfigData::from_json_str(json).unwrap()
    }

    #[test]
    fn test_validate_model_references_complete_match() {
        let tmp = TempDir::new().unwrap();
        let creds_json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"},
            "anthropic": {"provider":"anthropic","apiKey":"sk-ant"}
        }}"#;
        let creds = CredentialsProvider::from_json_str(creds_json).unwrap();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": { "models": [{"id":"gpt-4"}] },
                "anthropic": { "models": [{"id":"claude-3"}] }
            }
        }"#,
        );
        assert!(creds.validate_model_references(&models, tmp.path()).is_ok());
    }

    #[test]
    fn test_validate_model_references_missing_credential() {
        let tmp = TempDir::new().unwrap();
        let creds_json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"}
        }}"#;
        let creds = CredentialsProvider::from_json_str(creds_json).unwrap();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": { "models": [{"id":"gpt-4"}] },
                "anthropic": { "models": [{"id":"claude-3"}] }
            }
        }"#,
        );
        let result = creds.validate_model_references(&models, tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field.contains("anthropic")),
            "error should reference the missing provider: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_model_references_extra_credentials_only_warn() {
        let tmp = TempDir::new().unwrap();
        let creds_json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"},
            "backup": {"provider":"backup","apiKey":"sk-backup"}
        }}"#;
        let creds = CredentialsProvider::from_json_str(creds_json).unwrap();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": { "models": [{"id":"gpt-4"}] }
            }
        }"#,
        );
        assert!(creds.validate_model_references(&models, tmp.path()).is_ok());
    }

    #[test]
    fn test_validate_model_references_empty_models() {
        let tmp = TempDir::new().unwrap();
        let creds_json = r#"{"providers":{
            "openai": {"provider":"openai","apiKey":"sk-test"}
        }}"#;
        let creds = CredentialsProvider::from_json_str(creds_json).unwrap();
        let models = models_from_json(r#"{"providers": {}}"#);
        assert!(creds.validate_model_references(&models, tmp.path()).is_ok());
    }

    #[test]
    fn test_validate_model_references_empty_credentials_with_models() {
        let tmp = TempDir::new().unwrap();
        let creds = CredentialsProvider::default();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": { "models": [{"id":"gpt-4"}] }
            }
        }"#,
        );
        let result = creds.validate_model_references(&models, tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_model_references_empty_both() {
        let tmp = TempDir::new().unwrap();
        let creds = CredentialsProvider::default();
        let models = models_from_json(r#"{"providers": {}}"#);
        assert!(creds.validate_model_references(&models, tmp.path()).is_ok());
    }

    #[test]
    fn test_validate_model_references_credential_path_valid() {
        let tmp = TempDir::new().unwrap();
        // Create a credential file at credentials/openai.json
        let creds_dir = tmp.path().join("credentials");
        std::fs::create_dir_all(&creds_dir).unwrap();
        std::fs::write(
            creds_dir.join("openai.json"),
            r#"{"provider":"openai","apiKey":"sk-test"}"#,
        )
        .unwrap();

        let creds = CredentialsProvider::default();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": {
                    "credentialPath": "credentials/openai.json",
                    "models": [{"id":"gpt-4"}]
                }
            }
        }"#,
        );
        assert!(creds.validate_model_references(&models, tmp.path()).is_ok());
    }

    #[test]
    fn test_validate_model_references_credential_path_missing_file() {
        let tmp = TempDir::new().unwrap();
        // No credential file exists
        let creds = CredentialsProvider::default();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": {
                    "credentialPath": "credentials/openai.json",
                    "models": [{"id":"gpt-4"}]
                }
            }
        }"#,
        );
        let result = creds.validate_model_references(&models, tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field.contains("openai")),
            "error should reference the missing provider: {:?}",
            err
        );
    }

    #[test]
    fn test_validate_model_references_credential_path_empty() {
        let tmp = TempDir::new().unwrap();
        let creds = CredentialsProvider::default();
        let models = models_from_json(
            r#"{
            "providers": {
                "openai": {
                    "credentialPath": "",
                    "models": [{"id":"gpt-4"}]
                }
            }
        }"#,
        );
        let result = creds.validate_model_references(&models, tmp.path());
        assert!(result.is_err());
    }
}
