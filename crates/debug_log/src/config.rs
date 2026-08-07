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
    pub fn from_file(path: &std::path::Path) -> Result<Self, DebugLogConfigError> {
        let content = std::fs::read_to_string(path).map_err(DebugLogConfigError::Io)?;
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
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn sample_config() -> &'static str {
        r#"{
            "min_level": "info",
            "log_dir": "/tmp/debug-logs",
            "retention_days": 14,
            "redaction_patterns": [
                {"field": "api_key", "match_type": "exact", "replacement": "[REDACTED]"}
            ]
        }"#
    }

    #[test]
    fn test_deserialize_full_config() {
        let config: DebugLogConfig = serde_json::from_str(sample_config()).unwrap();
        assert_eq!(config.min_level, LogLevel::Info);
        assert_eq!(config.log_dir, PathBuf::from("/tmp/debug-logs"));
        assert_eq!(config.retention_days, 14);
        assert_eq!(config.redaction_patterns.len(), 1);
        assert_eq!(config.redaction_patterns[0].field, "api_key");
    }

    #[test]
    fn test_defaults() {
        let json = r#"{"log_dir": "/tmp/logs"}"#;
        let config: DebugLogConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.min_level, LogLevel::Debug);
        assert_eq!(config.retention_days, 7);
        assert!(config.redaction_patterns.is_empty());
    }

    #[test]
    fn test_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "{}", sample_config()).unwrap();
        let config = DebugLogConfig::from_file(file.path()).unwrap();
        assert_eq!(config.min_level, LogLevel::Info);
        assert_eq!(config.log_dir, PathBuf::from("/tmp/debug-logs"));
    }
}
