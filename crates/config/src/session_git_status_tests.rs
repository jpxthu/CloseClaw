//! Integration tests for session config git_status switch

use tempfile::TempDir;

use crate::session::{JsonSessionConfigProvider, PerAgentSessionConfig, SessionConfigProvider};
use closeclaw_common::AgentRole;

/// Write JSON content to a temp file and return its path.
fn write_temp_json(content: &str) -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("session_config.json");
    std::fs::write(&path, content).unwrap();
    (temp, path)
}

/// Default PerAgentSessionConfig must have is_git_status_enabled = false.
#[test]
fn test_default_per_agent_config_git_status_is_false() {
    let cfg = PerAgentSessionConfig::default();
    assert!(
        !cfg.is_git_status_enabled,
        "default is_git_status_enabled should be false"
    );
}

/// Config JSON without git_status field should deserialize with default false.
#[test]
fn test_config_without_git_status_field_uses_default() {
    let json = r#"{
        "defaults": {
            "mainAgent": { "idleMinutes": 10, "purgeAfterMinutes": 60 }
        },
        "sweeperIntervalSeconds": 300
    }"#;
    let (_temp, path) = write_temp_json(json);
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert!(
        !cfg.is_git_status_enabled,
        "missing git_status field should default to false"
    );
}

/// Config JSON with git_status: true should deserialize correctly.
#[test]
fn test_config_with_git_status_true() {
    let json = r#"{
        "defaults": {
            "mainAgent": { "idleMinutes": 10, "purgeAfterMinutes": 60, "gitStatus": true }
        },
        "sweeperIntervalSeconds": 300
    }"#;
    let (_temp, path) = write_temp_json(json);
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert!(
        cfg.is_git_status_enabled,
        "git_status: true should be parsed correctly"
    );
}

/// Config JSON with git_status: false explicitly should work.
#[test]
fn test_config_with_git_status_false() {
    let json = r#"{
        "defaults": {
            "mainAgent": { "idleMinutes": 10, "purgeAfterMinutes": 60, "gitStatus": false }
        },
        "sweeperIntervalSeconds": 300
    }"#;
    let (_temp, path) = write_temp_json(json);
    let provider = JsonSessionConfigProvider::new(&path).unwrap();
    let cfg = provider.session_config_for("any-agent", AgentRole::MainAgent);
    assert!(
        !cfg.is_git_status_enabled,
        "git_status: false should be parsed correctly"
    );
}

/// Serialization round-trip: git_status field must serialize/deserialize
/// using camelCase "gitStatus".
#[test]
fn test_git_status_serialization_round_trip() {
    let cfg = PerAgentSessionConfig::new(30, 0, true);
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(
        json.contains("gitStatus"),
        "should use camelCase gitStatus: {}",
        json
    );
    let parsed: PerAgentSessionConfig = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_git_status_enabled);
}
