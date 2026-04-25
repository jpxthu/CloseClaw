//! Gateway JSON ConfigProvider
//!
//! Loads and validates gateway.json configuration.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

/// Default port for the gateway server
pub const DEFAULT_PORT: u16 = 3000;
/// Default request timeout in milliseconds
pub const DEFAULT_TIMEOUT: u64 = 30000;
/// Default rate limit per minute
pub const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 60;
/// Default max message size in bytes
pub const DEFAULT_MAX_MESSAGE_SIZE: usize = 16384;
/// Default DM session scope
pub const DEFAULT_DM_SCOPE: &str = "per-channel-peer";

/// Gateway configuration data structure
///
/// Maps to the `gateway` field in `openclaw.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfigData {
    #[serde(default = "default_version")]
    pub version: String,

    pub name: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_timeout")]
    pub timeout: u64,

    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,

    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,

    #[serde(default = "default_dm_scope")]
    pub dm_scope: String,
}

fn default_version() -> String {
    "1.0.0".to_string()
}
fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT
}
fn default_rate_limit_per_minute() -> u32 {
    DEFAULT_RATE_LIMIT_PER_MINUTE
}
fn default_max_message_size() -> usize {
    DEFAULT_MAX_MESSAGE_SIZE
}
fn default_dm_scope() -> String {
    DEFAULT_DM_SCOPE.to_string()
}

impl Default for GatewayConfigData {
    fn default() -> Self {
        Self {
            version: default_version(),
            name: "closeclaw".to_string(),
            port: default_port(),
            timeout: default_timeout(),
            rate_limit_per_minute: default_rate_limit_per_minute(),
            max_message_size: default_max_message_size(),
            dm_scope: default_dm_scope(),
        }
    }
}

impl GatewayConfigData {
    /// Create a new provider from file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Create a new provider from a JSON string (useful for testing)
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: GatewayConfigData = serde_json::from_str(content)?;
        Ok(config)
    }
}

impl ConfigProvider for GatewayConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.port == 0 {
            return Err(ConfigError::ValueError {
                field: "port".to_string(),
                message: "port must be greater than 0".to_string(),
            });
        }
        // u16 max is 65535, so no upper-bound check needed
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "gateway.json"
    }

    fn is_default(&self) -> bool {
        self.version == default_version()
            && self.name == "closeclaw"
            && self.port == default_port()
            && self.timeout == default_timeout()
            && self.rate_limit_per_minute == default_rate_limit_per_minute()
            && self.max_message_size == default_max_message_size()
            && self.dm_scope == default_dm_scope()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn default_config() -> GatewayConfigData {
        GatewayConfigData::default()
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

    #[test]
    fn test_non_default_config() {
        let mut config = default_config();
        config.port = 8080;
        assert!(
            !config.is_default(),
            "modified config should return false from is_default()"
        );
    }

    // -------------------------------------------------------------------------
    // Port validation tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_port_zero_fails() {
        let mut config = default_config();
        config.port = 0;
        let result = config.validate();
        assert!(result.is_err(), "port=0 should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { field, .. } if field == "port"),
            "error should be about port field"
        );
    }

    #[test]
    fn test_port_max_valid() {
        let mut config = default_config();
        config.port = 65535;
        config.validate().expect("port=65535 should be valid");
    }

    #[test]
    fn test_port_1_valid() {
        let mut config = default_config();
        config.port = 1;
        config.validate().expect("port=1 should be valid");
    }

    // -------------------------------------------------------------------------
    // from_json_str tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_json_str_valid() {
        let json = r#"{
            "version": "1.0.0",
            "name": "my-gateway",
            "port": 3001,
            "timeout": 5000,
            "rateLimitPerMinute": 120,
            "maxMessageSize": 32768,
            "dmScope": "per-peer"
        }"#;
        let config = GatewayConfigData::from_json_str(json).expect("valid JSON should parse");
        assert_eq!(config.name, "my-gateway");
        assert_eq!(config.port, 3001);
        assert_eq!(config.timeout, 5000);
        assert_eq!(config.rate_limit_per_minute, 120);
        assert_eq!(config.max_message_size, 32768);
        assert_eq!(config.dm_scope, "per-peer");
    }

    #[test]
    fn test_from_json_str_missing_optional_fields() {
        let json = r#"{
            "name": "minimal-gateway"
        }"#;
        let config = GatewayConfigData::from_json_str(json)
            .expect("JSON with required fields only should parse");
        assert_eq!(config.name, "minimal-gateway");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
        assert_eq!(config.rate_limit_per_minute, DEFAULT_RATE_LIMIT_PER_MINUTE);
        assert_eq!(config.max_message_size, DEFAULT_MAX_MESSAGE_SIZE);
        assert_eq!(config.dm_scope, DEFAULT_DM_SCOPE);
    }

    #[test]
    fn test_from_json_str_invalid_json() {
        let json = "not valid json at all";
        let result = GatewayConfigData::from_json_str(json);
        assert!(result.is_err(), "invalid JSON should return Err");
    }

    #[test]
    fn test_from_json_str_empty_string() {
        let result = GatewayConfigData::from_json_str("");
        assert!(result.is_err(), "empty string should return Err");
    }

    // -------------------------------------------------------------------------
    // is_default edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_default_name_changed() {
        let mut config = default_config();
        config.name = "other".to_string();
        assert!(!config.is_default());
    }

    #[test]
    fn test_is_default_version_changed() {
        let mut config = default_config();
        config.version = "2.0.0".to_string();
        assert!(!config.is_default());
    }

    #[test]
    fn test_is_default_timeout_changed() {
        let mut config = default_config();
        config.timeout = 99999;
        assert!(!config.is_default());
    }

    #[test]
    fn test_is_default_rate_limit_changed() {
        let mut config = default_config();
        config.rate_limit_per_minute = 1;
        assert!(!config.is_default());
    }

    #[test]
    fn test_is_default_max_message_size_changed() {
        let mut config = default_config();
        config.max_message_size = 1;
        assert!(!config.is_default());
    }

    #[test]
    fn test_is_default_dm_scope_changed() {
        let mut config = default_config();
        config.dm_scope = "main".to_string();
        assert!(!config.is_default());
    }

    // -------------------------------------------------------------------------
    // config_path and version
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_path() {
        assert_eq!(GatewayConfigData::config_path(), "gateway.json");
    }

    #[test]
    fn test_version() {
        let config = default_config();
        assert_eq!(config.version(), "1.0.0");
    }
}
