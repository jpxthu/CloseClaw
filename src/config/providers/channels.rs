//! Channels JSON ConfigProvider
//!
//! Loads and validates config/channels.json configuration.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::providers::ConfigError;
use crate::config::ConfigProvider;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Binding match criteria — which channel and account this binding applies to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BindingMatch {
    pub channel: String,

    #[serde(rename = "accountId", default)]
    pub account_id: String,
}

/// A single binding entry — maps an agent to a specific channel + account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BindingEntry {
    pub agent_id: String,

    #[serde(rename = "match")]
    pub match_val: BindingMatch,
}

/// Root channels configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsConfigData {
    #[serde(default)]
    pub channels: HashMap<String, serde_json::Value>,

    #[serde(default)]
    pub bindings: Vec<BindingEntry>,
}

impl Default for ChannelsConfigData {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
            bindings: Vec::new(),
        }
    }
}

/// Allowed channel types in the system.
const ALLOWED_CHANNEL_TYPES: &[&str] = &[
    "feishu",
    "discord",
    "telegram",
    "slack",
    "whatsapp",
    "signal",
    "matrix",
    "msteams",
    "mattermost",
    "nostr",
    "nextcloud-talk",
    "synology-chat",
    "line",
    "googlechat",
    "bluebubbles",
    "imessage",
    "irc",
    "qqbot",
    "twitch",
    "openclaw",
];

impl ChannelsConfigData {
    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: ChannelsConfigData = serde_json::from_str(content)?;
        Ok(config)
    }

    /// Return channel keys that have `enabled = true`.
    pub fn enabled_channels(&self) -> Vec<&str> {
        self.channels
            .iter()
            .filter(|(_, v)| v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false))
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Get a channel config value by channel type.
    pub fn get_channel(&self, channel_type: &str) -> Option<&serde_json::Value> {
        self.channels.get(channel_type)
    }

    /// Return all bindings whose match.channel equals the given channel type.
    pub fn get_bindings_by_channel(&self, channel_type: &str) -> Vec<&BindingEntry> {
        self.bindings
            .iter()
            .filter(|b| b.match_val.channel == channel_type)
            .collect()
    }

    /// Return all bindings whose match.account_id equals the given account id.
    pub fn get_bindings_by_account(&self, account_id: &str) -> Vec<&BindingEntry> {
        self.bindings
            .iter()
            .filter(|b| b.match_val.account_id == account_id)
            .collect()
    }
}

impl ConfigProvider for ChannelsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // Validate channel types are in the allowed list
        for channel_type in self.channels.keys() {
            if !ALLOWED_CHANNEL_TYPES.contains(&channel_type.as_str()) {
                return Err(ConfigError::ValueError {
                    field: "channel_type".to_string(),
                    message: format!(
                        "unknown channel type '{}'. Allowed: {}",
                        channel_type,
                        ALLOWED_CHANNEL_TYPES.join(", ")
                    ),
                });
            }
        }

        // Validate binding fields are non-empty
        for (i, binding) in self.bindings.iter().enumerate() {
            if binding.agent_id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: format!("bindings[{}].agentId", i),
                    message: "agent_id cannot be empty".to_string(),
                });
            }

            if binding.match_val.channel.is_empty() {
                return Err(ConfigError::ValueError {
                    field: format!("bindings[{}].match.channel", i),
                    message: "match.channel cannot be empty".to_string(),
                });
            }

            if binding.match_val.account_id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: format!("bindings[{}].match.accountId", i),
                    message: "match.accountId cannot be empty".to_string(),
                });
            }
        }

        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "channels.json"
    }

    fn is_default(&self) -> bool {
        self.channels.is_empty() && self.bindings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn default_config() -> ChannelsConfigData {
        ChannelsConfigData::default()
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
    fn test_validate_unknown_channel_type() {
        let json = r#"{
            "channels": {
                "unknown-channel": { "enabled": true }
            }
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(
            result.is_err(),
            "unknown channel type should fail validation"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field == "channel_type"),
            "error should be about channel_type field"
        );
    }

    #[test]
    fn test_validate_empty_binding_agent_id() {
        let json = r#"{
            "bindings": [
                { "agentId": "", "match": { "channel": "feishu", "accountId": "acc1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(result.is_err(), "empty agent_id should fail validation");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field.contains("agentId")),
            "error should be about agentId field"
        );
    }

    #[test]
    fn test_validate_empty_binding_channel() {
        let json = r#"{
            "bindings": [
                { "agentId": "agent-1", "match": { "channel": "", "accountId": "acc1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(
            result.is_err(),
            "empty match.channel should fail validation"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field.contains("channel")),
            "error should be about match.channel field"
        );
    }

    #[test]
    fn test_validate_empty_binding_account_id() {
        let json = r#"{
            "bindings": [
                { "agentId": "agent-1", "match": { "channel": "feishu", "accountId": "" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let result = config.validate();
        assert!(
            result.is_err(),
            "empty match.accountId should fail validation"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ConfigError::ValueError { ref field, .. } if field.contains("accountId")),
            "error should be about match.accountId field"
        );
    }

    #[test]
    fn test_validate_valid_config() {
        let json = r#"{
            "channels": {
                "feishu": { "enabled": true, "appId": "cli_xxx", "accounts": ["acc1"] }
            },
            "bindings": [
                { "agentId": "agent-1", "match": { "channel": "feishu", "accountId": "acc1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        config.validate().expect("valid config should pass");
    }

    // -------------------------------------------------------------------------
    // from_json_str tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_from_json_str_full() {
        let json = r#"{
            "channels": {
                "feishu": { "enabled": true, "appId": "cli_xxx", "accounts": ["acc1"] },
                "telegram": { "enabled": false, "token": "bot-token" }
            },
            "bindings": [
                { "agentId": "main", "match": { "channel": "feishu", "accountId": "acc1" } },
                { "agentId": "sub", "match": { "channel": "telegram", "accountId": "bot1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).expect("valid JSON should parse");
        assert_eq!(config.channels.len(), 2);
        assert!(config.channels.contains_key("feishu"));
        assert!(config.channels.contains_key("telegram"));
        assert_eq!(config.bindings.len(), 2);
        assert_eq!(config.bindings[0].agent_id, "main");
        assert_eq!(config.bindings[0].match_val.channel, "feishu");
        assert_eq!(config.bindings[0].match_val.account_id, "acc1");
        assert_eq!(config.bindings[1].agent_id, "sub");
        assert_eq!(config.bindings[1].match_val.channel, "telegram");
    }

    #[test]
    fn test_from_json_str_minimal() {
        let json = r#"{}"#;
        let config = ChannelsConfigData::from_json_str(json).expect("empty JSON should parse");
        assert!(config.channels.is_empty());
        assert!(config.bindings.is_empty());
    }

    #[test]
    fn test_from_json_str_invalid_json() {
        let result = ChannelsConfigData::from_json_str("not json at all");
        assert!(result.is_err(), "invalid JSON should return Err");
    }

    #[test]
    fn test_from_json_str_empty_string() {
        let result = ChannelsConfigData::from_json_str("");
        assert!(result.is_err(), "empty string should return Err");
    }

    // -------------------------------------------------------------------------
    // query interface tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_enabled_channels() {
        let json = r#"{
            "channels": {
                "feishu": { "enabled": true },
                "telegram": { "enabled": false },
                "discord": { "enabled": true },
                "slack": {}
            }
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let enabled = config.enabled_channels();
        assert!(enabled.contains(&"feishu"));
        assert!(!enabled.contains(&"telegram"));
        assert!(enabled.contains(&"discord"));
        assert!(!enabled.contains(&"slack"));
    }

    #[test]
    fn test_get_channel_hit() {
        let json = r#"{
            "channels": {
                "feishu": { "appId": "cli_xxx" }
            }
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        assert!(config.get_channel("feishu").is_some());
        let val = config.get_channel("feishu").unwrap();
        assert_eq!(val.get("appId").and_then(|x| x.as_str()), Some("cli_xxx"));
    }

    #[test]
    fn test_get_channel_miss() {
        let config = default_config();
        assert!(config.get_channel("nonexistent").is_none());
    }

    #[test]
    fn test_get_bindings_by_channel() {
        let json = r#"{
            "bindings": [
                { "agentId": "a1", "match": { "channel": "feishu", "accountId": "acc1" } },
                { "agentId": "a2", "match": { "channel": "feishu", "accountId": "acc2" } },
                { "agentId": "a3", "match": { "channel": "telegram", "accountId": "acc1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let feishu_bindings = config.get_bindings_by_channel("feishu");
        assert_eq!(feishu_bindings.len(), 2);
        let telegram_bindings = config.get_bindings_by_channel("telegram");
        assert_eq!(telegram_bindings.len(), 1);
        let nonexistent = config.get_bindings_by_channel("nonexistent");
        assert!(nonexistent.is_empty());
    }

    #[test]
    fn test_get_bindings_by_account() {
        let json = r#"{
            "bindings": [
                { "agentId": "a1", "match": { "channel": "feishu", "accountId": "acc1" } },
                { "agentId": "a2", "match": { "channel": "telegram", "accountId": "acc1" } },
                { "agentId": "a3", "match": { "channel": "feishu", "accountId": "acc2" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        let acc1_bindings = config.get_bindings_by_account("acc1");
        assert_eq!(acc1_bindings.len(), 2);
        let acc2_bindings = config.get_bindings_by_account("acc2");
        assert_eq!(acc2_bindings.len(), 1);
        let nonexistent = config.get_bindings_by_account("nonexistent");
        assert!(nonexistent.is_empty());
    }

    // -------------------------------------------------------------------------
    // is_default tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_default_channels_not_empty() {
        let json = r#"{
            "channels": { "feishu": {} }
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "non-empty channels should not be default"
        );
    }

    #[test]
    fn test_is_default_bindings_not_empty() {
        let json = r#"{
            "bindings": [
                { "agentId": "a1", "match": { "channel": "feishu", "accountId": "acc1" } }
            ]
        }"#;
        let config = ChannelsConfigData::from_json_str(json).unwrap();
        assert!(
            !config.is_default(),
            "non-empty bindings should not be default"
        );
    }

    // -------------------------------------------------------------------------
    // config_path and version
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_path() {
        assert_eq!(ChannelsConfigData::config_path(), "channels.json");
    }

    #[test]
    fn test_version() {
        let config = default_config();
        assert_eq!(config.version(), "1.0.0");
    }
}
