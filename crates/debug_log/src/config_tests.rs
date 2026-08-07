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

#[tokio::test]
async fn test_from_file() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "{}", sample_config()).unwrap();
    let config = DebugLogConfig::from_file(file.path()).await.unwrap();
    assert_eq!(config.min_level, LogLevel::Info);
    assert_eq!(config.log_dir, PathBuf::from("/tmp/debug-logs"));
}
