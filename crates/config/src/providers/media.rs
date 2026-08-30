//! Media configuration provider.
//!
//! Loads and validates `media.json` which controls media storage
//! parameters: storage directory, retention period, and image content
//! threshold.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::providers::ConfigError;
use crate::ConfigProvider;

/// Current config version for media configuration.
pub const CURRENT_VERSION: &str = "1.0.0";

/// Default storage directory for media files.
const DEFAULT_STORAGE_DIR: &str = "~/.closeclaw/media";
/// Default retention period in days (7 days).
const DEFAULT_RETENTION_DAYS: u64 = 7;
/// Default image content threshold in bytes (1 MB).
const DEFAULT_IMAGE_CONTENT_THRESHOLD: u64 = 1_048_576;

/// Media configuration data.
///
/// Parsed from `media.json` and provides runtime parameters for the
/// media storage subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaConfigData {
    /// Config version.
    #[serde(default = "default_version")]
    pub version: String,

    /// Directory path for media file storage.
    /// Supports `~` prefix for home directory expansion.
    #[serde(default = "default_storage_dir")]
    pub storage_dir: String,

    /// Number of days to retain media files.
    /// Set to 0 to disable periodic cleanup.
    #[serde(default = "default_retention_days")]
    pub retention_days: u64,

    /// Image content threshold in bytes.
    /// Images exceeding this size enter context as media references
    /// rather than inline content.
    #[serde(default = "default_image_content_threshold")]
    pub image_content_threshold_bytes: u64,
}

fn default_version() -> String {
    CURRENT_VERSION.to_string()
}

fn default_storage_dir() -> String {
    DEFAULT_STORAGE_DIR.to_string()
}

fn default_retention_days() -> u64 {
    DEFAULT_RETENTION_DAYS
}

fn default_image_content_threshold() -> u64 {
    DEFAULT_IMAGE_CONTENT_THRESHOLD
}

impl Default for MediaConfigData {
    fn default() -> Self {
        Self {
            version: default_version(),
            storage_dir: default_storage_dir(),
            retention_days: default_retention_days(),
            image_content_threshold_bytes: default_image_content_threshold(),
        }
    }
}

impl MediaConfigData {
    /// Parse from a JSON string (useful for testing).
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let data: MediaConfigData = serde_json::from_str(content)?;
        Ok(data)
    }

    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }
}

impl ConfigProvider for MediaConfigData {
    fn version(&self) -> &'static str {
        CURRENT_VERSION
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.storage_dir.trim().is_empty() {
            return Err(ConfigError::ValueError {
                field: "storage_dir".to_string(),
                message: "storage_dir must not be empty".to_string(),
            });
        }
        // retention_days and image_content_threshold_bytes are u64,
        // so they are always >= 0 by type constraint.
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "media.json"
    }

    fn is_default(&self) -> bool {
        self.version == default_version()
            && self.storage_dir == DEFAULT_STORAGE_DIR
            && self.retention_days == DEFAULT_RETENTION_DAYS
            && self.image_content_threshold_bytes == DEFAULT_IMAGE_CONTENT_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = MediaConfigData::default();
        config.validate().expect("default config should be valid");
    }

    #[test]
    fn test_default_config_is_default() {
        let config = MediaConfigData::default();
        assert!(config.is_default());
    }

    #[test]
    fn test_non_default_config() {
        let mut config = MediaConfigData::default();
        config.retention_days = 14;
        assert!(!config.is_default());
    }

    #[test]
    fn test_validate_empty_storage_dir() {
        let mut config = MediaConfigData::default();
        config.storage_dir = String::new();
        let result = config.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::ValueError { field, .. } => assert_eq!(field, "storage_dir"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_validate_whitespace_only_storage_dir() {
        let mut config = MediaConfigData::default();
        config.storage_dir = "   ".to_string();
        let result = config.validate();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigError::ValueError { field, .. } => assert_eq!(field, "storage_dir"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn test_retention_days_zero_is_valid() {
        let mut config = MediaConfigData::default();
        config.retention_days = 0;
        config.validate().expect("retention_days=0 should be valid");
    }

    #[test]
    fn test_image_threshold_zero_is_valid() {
        let mut config = MediaConfigData::default();
        config.image_content_threshold_bytes = 0;
        config.validate().expect("threshold=0 should be valid");
    }

    #[test]
    fn test_from_json_str_full() {
        let json = r#"{
            "version": "1.0.0",
            "storageDir": "/data/media",
            "retentionDays": 30,
            "imageContentThresholdBytes": 2097152
        }"#;
        let config = MediaConfigData::from_json_str(json).expect("valid JSON should parse");
        assert_eq!(config.storage_dir, "/data/media");
        assert_eq!(config.retention_days, 30);
        assert_eq!(config.image_content_threshold_bytes, 2_097_152);
    }

    #[test]
    fn test_from_json_str_defaults() {
        let json = r#"{"storageDir": "/tmp/media"}"#;
        let config = MediaConfigData::from_json_str(json)
            .expect("JSON with only required fields should parse");
        assert_eq!(config.storage_dir, "/tmp/media");
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(
            config.image_content_threshold_bytes,
            DEFAULT_IMAGE_CONTENT_THRESHOLD
        );
    }

    #[test]
    fn test_from_json_str_empty_object() {
        let json = "{}";
        let config =
            MediaConfigData::from_json_str(json).expect("empty object should use all defaults");
        assert_eq!(config.storage_dir, DEFAULT_STORAGE_DIR);
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(
            config.image_content_threshold_bytes,
            DEFAULT_IMAGE_CONTENT_THRESHOLD
        );
    }

    #[test]
    fn test_from_json_str_invalid() {
        let result = MediaConfigData::from_json_str("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_config_path() {
        assert_eq!(MediaConfigData::config_path(), "media.json");
    }
}
