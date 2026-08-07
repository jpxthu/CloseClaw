use std::path::PathBuf;

use serde::Deserialize;

use crate::{LogLevel, RedactionPattern};

/// Configuration for the debug log framework.
///
/// Loaded from a JSON config file and passed to `DebugLog::new()`.
/// Uses struct injection — no global state.
#[derive(Debug, Clone, Deserialize)]
pub struct DebugLogConfig {
    /// Minimum severity level to record (default: `Debug`).
    #[serde(default = "default_min_level")]
    pub min_level: LogLevel,
    /// Directory for log files.
    pub log_dir: PathBuf,
    /// Number of days to retain log files before cleanup.
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    /// Sensitive field patterns for credential redaction.
    #[serde(default)]
    pub redaction_patterns: Vec<RedactionPattern>,
}

fn default_min_level() -> LogLevel {
    LogLevel::Debug
}

fn default_retention_days() -> u32 {
    7
}

impl DebugLogConfig {
    /// Load configuration from a JSON file path.
    pub async fn from_file(path: &std::path::Path) -> Result<Self, DebugLogConfigError> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(DebugLogConfigError::Io)?;
        let config = serde_json::from_str(&content).map_err(DebugLogConfigError::Parse)?;
        Ok(config)
    }
}

/// Errors that can occur when loading a `DebugLogConfig`.
#[derive(Debug, thiserror::Error)]
pub enum DebugLogConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
