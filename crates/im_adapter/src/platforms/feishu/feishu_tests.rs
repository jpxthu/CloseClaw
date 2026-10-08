//! Unit tests for feishu platform configuration (Step 1.7 dimensions).
//!
//! Covers config-driven platform enablement: enabled, disabled, not
//! listed, missing config file, invalid JSON.
//!
//! Identity mapping assembly (`accounts.json` → resolver) lives at the
//! composition root and is covered by the daemon's platform injection
//! tests.

use super::config::load_platforms_config;
use std::fs;
use tempfile::TempDir;

// =========================================================================
// Helper: create a temp config_dir with a given platforms.json content
// =========================================================================

fn setup_config_dir(platforms_json: Option<&str>) -> TempDir {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config");
    fs::create_dir_all(&config).unwrap();
    if let Some(content) = platforms_json {
        fs::write(config.join("platforms.json"), content).unwrap();
    }
    dir
}

// =========================================================================
// Platform enablement tests (load_platforms_config)
// =========================================================================

/// Config exists and feishu is enabled → is_enabled returns true.
#[test]
fn test_platform_enabled_in_config() {
    let json = r#"{"platforms":{"feishu":{"enabled":true}}}"#;
    let dir = setup_config_dir(Some(json));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(cfg.is_enabled("feishu"));
}

/// Config exists but feishu is disabled → is_enabled returns false.
#[test]
fn test_platform_disabled_in_config() {
    let json = r#"{"platforms":{"feishu":{"enabled":false}}}"#;
    let dir = setup_config_dir(Some(json));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
}

/// Config exists but feishu is not listed → is_enabled returns false.
#[test]
fn test_platform_not_listed_in_config() {
    let json = r#"{"platforms":{"discord":{"enabled":true}}}"#;
    let dir = setup_config_dir(Some(json));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
}

/// Config file missing → all platforms disabled (default).
#[test]
fn test_platform_config_missing() {
    let dir = setup_config_dir(None);
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
    assert!(!cfg.is_enabled("discord"));
}

/// Config file contains invalid JSON → all platforms disabled.
#[test]
fn test_platform_config_invalid_json() {
    let dir = setup_config_dir(Some("not valid json"));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
}

/// Empty JSON object → no platforms enabled.
#[test]
fn test_platform_config_empty_object() {
    let dir = setup_config_dir(Some("{}"));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
}

/// Multiple platforms, each independently configurable.
#[test]
fn test_platform_config_multiple_platforms() {
    let json = r#"{"platforms":{"feishu":{"enabled":true},"discord":{"enabled":false},"slack":{"enabled":true}}}"#;
    let dir = setup_config_dir(Some(json));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(cfg.is_enabled("feishu"));
    assert!(!cfg.is_enabled("discord"));
    assert!(cfg.is_enabled("slack"));
}

/// Default-enabled platform entry (missing `enabled` field defaults to false).
#[test]
fn test_platform_config_default_not_enabled() {
    let json = r#"{"platforms":{"feishu":{}}}"#;
    let dir = setup_config_dir(Some(json));
    let cfg = load_platforms_config(dir.path().to_str().unwrap());
    assert!(!cfg.is_enabled("feishu"));
}
