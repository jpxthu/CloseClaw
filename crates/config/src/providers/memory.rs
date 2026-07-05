//! Global memory configuration provider.
//!
//! Loads and validates the top-level `memory.json` config file that
//! provides global defaults for the memory subsystem. Per-agent configs
//! can override individual fields; see `resolved.rs` for the merge logic.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agents::MemoryConfig;
use crate::providers::ConfigError;
use crate::ConfigProvider;

/// Wrapper around [`MemoryConfig`] that implements the [`ConfigProvider`] trait.
///
/// The global `memory.json` has the same schema as the per-agent `memory`
/// section.  All features default to `disabled` when the file is absent
/// or empty (matching `MemoryConfig::default()`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryConfigData {
    #[serde(flatten)]
    pub config: MemoryConfig,
}

impl MemoryConfigData {
    /// Parse from a JSON string (useful for testing).
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let data: MemoryConfigData = serde_json::from_str(content)?;
        Ok(data)
    }

    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }
}

impl ConfigProvider for MemoryConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // Structural validation: serde already ensures required fields
        // are present with correct types. Additional business rules
        // (e.g., non-negative thresholds) can be added here.
        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "memory.json"
    }

    fn is_default(&self) -> bool {
        !self.config.mining.enabled.unwrap_or(false)
            && !self.config.dreaming.enabled.unwrap_or(false)
            && !self.config.search.enabled.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_valid() {
        let data = MemoryConfigData::default();
        data.validate().expect("default config should be valid");
    }

    #[test]
    fn test_default_is_default() {
        let data = MemoryConfigData::default();
        assert!(data.is_default());
    }

    #[test]
    fn test_config_path() {
        assert_eq!(MemoryConfigData::config_path(), "memory.json");
    }

    #[test]
    fn test_from_json_str_empty_object() {
        let data = MemoryConfigData::from_json_str("{}").expect("empty object should parse");
        assert!(data.is_default());
    }

    #[test]
    fn test_from_json_str_with_search_enabled() {
        let json = r#"{"search": {"enabled": true, "timeoutMs": 5000}}"#;
        let data = MemoryConfigData::from_json_str(json).expect("valid JSON should parse");
        assert!(data.config.search.enabled == Some(true));
        assert_eq!(data.config.search.timeout_ms, 5000);
        assert!(!data.is_default());
    }

    #[test]
    fn test_from_json_str_invalid() {
        let result = MemoryConfigData::from_json_str("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_file_nonexistent() {
        let result = MemoryConfigData::from_file("/nonexistent/memory.json");
        assert!(result.is_err());
    }
}
