//! Unit tests for feishu platform configuration and identity mapping.
//!
//! Covers the test dimensions from Step 1.7:
//! - Config-driven platform enablement: enabled + env present, enabled + env
//!   missing, disabled, missing config file
//! - Identity mapping: accounts.json loaded correctly, resolve returns
//!   expected account_id, missing file falls back to sender_id

use super::{load_identity_resolver, load_platforms_config};
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

fn setup_accounts_dir(accounts_json: Option<&str>) -> TempDir {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config");
    fs::create_dir_all(&config).unwrap();
    if let Some(content) = accounts_json {
        fs::write(config.join("accounts.json"), content).unwrap();
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

// =========================================================================
// Identity mapping tests (load_identity_resolver)
// =========================================================================

/// accounts.json exists with valid mappings → resolver loaded with correct
/// mappings.
#[test]
fn test_identity_mapping_loads_accounts_json() {
    let json = r#"{"accounts":[{"platform":"feishu","sender_id":"ou_aaa","account_id":"user1"},{"platform":"feishu","sender_id":"ou_bbb","account_id":"user2"}]}"#;
    let dir = setup_accounts_dir(Some(json));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap());
    assert!(resolver.is_some());
    let r = resolver.unwrap();
    assert_eq!(r.resolve("feishu", "ou_aaa"), Some("user1".into()));
    assert_eq!(r.resolve("feishu", "ou_bbb"), Some("user2".into()));
}

/// accounts.json missing → no resolver (returns None).
#[test]
fn test_identity_mapping_missing_file() {
    let dir = setup_accounts_dir(None);
    let resolver = load_identity_resolver(dir.path().to_str().unwrap());
    assert!(resolver.is_none());
}

/// accounts.json empty → no resolver (returns None).
#[test]
fn test_identity_mapping_empty_accounts() {
    let json = r#"{"accounts":[]}"#;
    let dir = setup_accounts_dir(Some(json));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap());
    assert!(resolver.is_none());
}

/// accounts.json with invalid JSON → no resolver.
#[test]
fn test_identity_mapping_invalid_json() {
    let dir = setup_accounts_dir(Some("not json"));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap());
    assert!(resolver.is_none());
}

/// Identity resolver: unknown sender_id returns None (fallback to raw sender_id
/// at call site).
#[test]
fn test_identity_mapping_unknown_sender() {
    let json = r#"{"accounts":[{"platform":"feishu","sender_id":"ou_xxx","account_id":"user1"}]}"#;
    let dir = setup_accounts_dir(Some(json));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(resolver.resolve("feishu", "ou_unknown"), None);
}

/// Identity resolver: cross-platform isolation (feishu mapping doesn't affect
/// discord).
#[test]
fn test_identity_mapping_cross_platform_isolation() {
    let json = r#"{"accounts":[{"platform":"feishu","sender_id":"ou_aaa","account_id":"user1"},{"platform":"discord","sender_id":"12345","account_id":"user2"}]}"#;
    let dir = setup_accounts_dir(Some(json));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(resolver.resolve("feishu", "ou_aaa"), Some("user1".into()));
    assert_eq!(resolver.resolve("discord", "12345"), Some("user2".into()));
    assert_eq!(resolver.resolve("discord", "ou_aaa"), None);
    assert_eq!(resolver.resolve("feishu", "12345"), None);
}

/// accounts.json with multiple accounts sharing the same account_id
/// (many-to-one mapping).
#[test]
fn test_identity_mapping_many_to_one() {
    let json = r#"{"accounts":[{"platform":"feishu","sender_id":"ou_aaa","account_id":"alice"},{"platform":"discord","sender_id":"99","account_id":"alice"},{"platform":"slack","sender_id":"U001","account_id":"alice"}]}"#;
    let dir = setup_accounts_dir(Some(json));
    let resolver = load_identity_resolver(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(resolver.resolve("feishu", "ou_aaa"), Some("alice".into()));
    assert_eq!(resolver.resolve("discord", "99"), Some("alice".into()));
    assert_eq!(resolver.resolve("slack", "U001"), Some("alice".into()));
}
