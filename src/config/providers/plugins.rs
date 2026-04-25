//! Plugins JSON ConfigProvider
//!
//! Loads and validates the plugins section of openclaw.json.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Plugin entry — enable/disable flag for a named plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    #[serde(default)]
    pub enabled: bool,
}

/// Installation metadata for a single plugin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstallInfo {
    #[serde(default)]
    pub source: Option<String>,

    #[serde(rename = "sourcePath", default)]
    pub source_path: Option<String>,

    #[serde(rename = "installPath", default)]
    pub install_path: Option<String>,

    #[serde(default)]
    pub version: Option<String>,

    #[serde(rename = "installedAt", default)]
    pub installed_at: Option<String>,
}

/// Root plugins configuration.
///
/// Maps to the `plugins` field in `openclaw.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginsConfigData {
    #[serde(default = "default_version")]
    pub version: String,

    #[serde(default = "default_enabled")]
    pub enabled: bool,

    #[serde(default)]
    pub allow: Vec<String>,

    #[serde(default)]
    pub entries: BTreeMap<String, PluginEntry>,

    #[serde(default)]
    pub installs: BTreeMap<String, PluginInstallInfo>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

fn default_enabled() -> bool {
    true
}

impl Default for PluginsConfigData {
    fn default() -> Self {
        Self {
            version: default_version(),
            enabled: default_enabled(),
            allow: Vec::new(),
            entries: BTreeMap::new(),
            installs: BTreeMap::new(),
        }
    }
}

impl PluginsConfigData {
    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: PluginsConfigData = serde_json::from_str(content)?;
        Ok(config)
    }
}

impl ConfigProvider for PluginsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.allow.iter().any(|name| name.is_empty()) {
            return Err(ConfigError::ValueError {
                field: "allow".to_string(),
                message: "plugin name in allow list cannot be empty".to_string(),
            });
        }
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "config/plugins.json"
    }

    fn is_default(&self) -> bool {
        self.version == default_version()
            && self.enabled == default_enabled()
            && self.allow.is_empty()
            && self.entries.is_empty()
            && self.installs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn default_config() -> PluginsConfigData {
        PluginsConfigData::default()
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
    // from_json_str tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_json_str_valid() {
        let json = r#"{
            "version": "1.0.0",
            "enabled": true,
            "allow": ["minimax", "openclaw-lark"],
            "entries": {
                "minimax": { "enabled": true },
                "feishu": { "enabled": false }
            },
            "installs": {
                "openclaw-lark": {
                    "source": "archive",
                    "sourcePath": "/tmp/lark.tgz",
                    "installPath": "/home/user/.openclaw/extensions/openclaw-lark",
                    "version": "2026.4.8",
                    "installedAt": "2026-04-19T12:42:35Z"
                }
            }
        }"#;
        let config = PluginsConfigData::from_json_str(json).expect("valid JSON should parse");
        assert!(config.enabled);
        assert_eq!(config.allow.len(), 2);
        assert_eq!(config.allow[0], "minimax");
        assert_eq!(config.entries["minimax"].enabled, true);
        assert_eq!(config.entries["feishu"].enabled, false);
        assert_eq!(
            config.installs["openclaw-lark"].source.as_deref(),
            Some("archive")
        );
        assert_eq!(
            config.installs["openclaw-lark"].source_path.as_deref(),
            Some("/tmp/lark.tgz")
        );
        assert_eq!(
            config.installs["openclaw-lark"].install_path.as_deref(),
            Some("/home/user/.openclaw/extensions/openclaw-lark")
        );
        assert_eq!(
            config.installs["openclaw-lark"].version.as_deref(),
            Some("2026.4.8")
        );
        assert_eq!(
            config.installs["openclaw-lark"].installed_at.as_deref(),
            Some("2026-04-19T12:42:35Z")
        );
    }

    #[test]
    fn test_from_json_str_minimal() {
        // Only "enabled" field — all others should get defaults
        let json = r#"{
            "enabled": false
        }"#;
        let config =
            PluginsConfigData::from_json_str(json).expect("JSON with enabled only should parse");
        assert!(!config.enabled);
        assert_eq!(config.version, default_version());
        assert!(config.allow.is_empty());
        assert!(config.entries.is_empty());
        assert!(config.installs.is_empty());
    }

    #[test]
    fn test_from_json_str_invalid_json() {
        let json = "not valid json at all";
        let result = PluginsConfigData::from_json_str(json);
        assert!(result.is_err(), "invalid JSON should return Err");
    }

    #[test]
    fn test_from_json_str_empty_string() {
        let result = PluginsConfigData::from_json_str("");
        assert!(result.is_err(), "empty string should return Err");
    }

    // -------------------------------------------------------------------------
    // validate tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_validate_empty_allow_name() {
        let json = r#"{
            "enabled": true,
            "allow": ["minimax", "", "openclaw-lark"]
        }"#;
        let config = PluginsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(
            result.is_err(),
            "empty plugin name in allow should fail validation"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::ValueError { ref field, .. } if field == "allow"
            ),
            "error should be about allow field"
        );
    }

    // -------------------------------------------------------------------------
    // is_default tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_default_with_entries() {
        let json = r#"{
            "enabled": true,
            "entries": { "minimax": { "enabled": true } }
        }"#;
        let config = PluginsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "config with entries should not be default"
        );
    }

    #[test]
    fn test_is_default_with_allow() {
        let json = r#"{
            "enabled": true,
            "allow": ["minimax"]
        }"#;
        let config = PluginsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "config with allow list should not be default"
        );
    }

    #[test]
    fn test_is_default_with_installs() {
        let json = r#"{
            "enabled": true,
            "installs": { "openclaw-lark": { "source": "archive" } }
        }"#;
        let config = PluginsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "config with installs should not be default"
        );
    }

    #[test]
    fn test_is_default_disabled() {
        let json = r#"{
            "enabled": false
        }"#;
        let config = PluginsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "config with enabled=false should not be default"
        );
    }

    // -------------------------------------------------------------------------
    // config_path and version
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_path() {
        assert_eq!(PluginsConfigData::config_path(), "config/plugins.json");
    }

    #[test]
    fn test_version() {
        let config = default_config();
        assert_eq!(config.version(), "1.0.0");
    }

    // -------------------------------------------------------------------------
    // PluginInstallInfo deserialize
    // -------------------------------------------------------------------------

    #[test]
    fn test_plugin_install_info_deserialize() {
        let json = r#"{
            "source": "archive",
            "sourcePath": "/tmp/plugin.tgz",
            "installPath": "/home/user/.openclaw/plugins/my-plugin",
            "version": "1.0.0",
            "installedAt": "2026-01-01T00:00:00Z"
        }"#;
        let info: PluginInstallInfo =
            serde_json::from_str(json).expect("valid PluginInstallInfo JSON should parse");
        assert_eq!(info.source.as_deref(), Some("archive"));
        assert_eq!(info.source_path.as_deref(), Some("/tmp/plugin.tgz"));
        assert_eq!(
            info.install_path.as_deref(),
            Some("/home/user/.openclaw/plugins/my-plugin")
        );
        assert_eq!(info.version.as_deref(), Some("1.0.0"));
        assert_eq!(info.installed_at.as_deref(), Some("2026-01-01T00:00:00Z"));
    }

    #[test]
    fn test_plugin_install_info_empty() {
        // All fields optional — empty object should deserialize fine
        let json = r#"{}"#;
        let info: PluginInstallInfo =
            serde_json::from_str(json).expect("empty object should parse");
        assert!(info.source.is_none());
        assert!(info.source_path.is_none());
        assert!(info.install_path.is_none());
        assert!(info.version.is_none());
        assert!(info.installed_at.is_none());
    }

    // -------------------------------------------------------------------------
    // PluginEntry deserialize
    // -------------------------------------------------------------------------

    #[test]
    fn test_plugin_entry_deserialize() {
        let json = r#"{"enabled": true}"#;
        let entry: PluginEntry =
            serde_json::from_str(json).expect("valid PluginEntry JSON should parse");
        assert!(entry.enabled);

        let json2 = r#"{"enabled": false}"#;
        let entry2: PluginEntry =
            serde_json::from_str(json2).expect("valid PluginEntry JSON should parse");
        assert!(!entry2.enabled);
    }

    #[test]
    fn test_plugin_entry_default() {
        let json = r#"{}"#;
        let entry: PluginEntry =
            serde_json::from_str(json).expect("empty object should parse as PluginEntry");
        assert!(!entry.enabled, "PluginEntry defaults to enabled=false");
    }
}
