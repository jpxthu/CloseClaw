//! Integration tests for session config compact functionality

use tempfile::TempDir;

use crate::config::session::{JsonSessionConfigProvider, SessionConfigProvider};
use crate::session::compaction::CompactConfig;

/// Minimal valid session config JSON with given defaults, agents, and sweeper interval.
fn valid_config_json(defaults: &str, agents: &str, sweeper_interval_secs: u64) -> String {
    format!(
        r#"{{"defaults":{},"agents":{},"sweeperIntervalSecs":{}}}"#,
        defaults, agents, sweeper_interval_secs
    )
}

/// Write JSON content to a temp file and return its path.
fn write_temp_json(content: &str) -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("session_config.json");
    std::fs::write(&path, content).unwrap();
    (temp, path)
}

#[test]
fn test_compact_config_parsed_when_present() {
    let json = r#"{"defaults":{"mainAgent":{"idleMinutes":10,"purgeAfterMinutes":60}},"agents":{},"sweeperIntervalSecs":300,"compact":{"charsPerToken":0.3,"autoCompactBufferTokens":15000,"maxConsecutiveFailures":5}}"#.to_string();
    let (_temp, path) = write_temp_json(&json);

    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    let compact = provider.compact_config();

    assert_eq!(compact.chars_per_token, 0.3);
    assert_eq!(compact.auto_compact_buffer_tokens, 15000);
    assert_eq!(compact.max_consecutive_failures, 5);
}

#[test]
fn test_compact_config_returns_default_when_absent() {
    let json = valid_config_json(
        r#"{"mainAgent":{"idleMinutes":10,"purgeAfterMinutes":60}}"#,
        "{}",
        300,
    );
    let (_temp, path) = write_temp_json(&json);

    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    let compact = provider.compact_config();
    let default_compact = CompactConfig::default();

    assert_eq!(compact.chars_per_token, default_compact.chars_per_token);
    assert_eq!(
        compact.auto_compact_buffer_tokens,
        default_compact.auto_compact_buffer_tokens
    );
    assert_eq!(
        compact.max_consecutive_failures,
        default_compact.max_consecutive_failures
    );
}

#[test]
fn test_compact_config_fallback_when_no_config_file() {
    let temp = TempDir::new().unwrap();
    let nonexistent = temp.path().join("nonexistent.json");
    let provider = JsonSessionConfigProvider::new(&nonexistent).unwrap();

    let compact = provider.compact_config();
    let default_compact = CompactConfig::default();

    assert_eq!(compact.chars_per_token, default_compact.chars_per_token);
    assert_eq!(
        compact.auto_compact_buffer_tokens,
        default_compact.auto_compact_buffer_tokens
    );
    assert_eq!(
        compact.max_consecutive_failures,
        default_compact.max_consecutive_failures
    );
}
