//! Tests for ConfigManager::with_default_root_dir()
//!
//! The error-path test (HOME not set) is covered by
//! `closeclaw_platform::config_tests::root_dir_inner` tests. These tests
//! verify the ConfigManager integration layer.

use super::*;

// ---------------------------------------------------------------------------
// Happy path — HOME is set → returns correct config directory
// ---------------------------------------------------------------------------

/// Test: with_default_root_dir() succeeds when HOME is set, producing a
/// config_dir that ends with `/.closeclaw/config`.
#[test]
fn test_with_default_root_dir_success() {
    // HOME is always set in a normal test environment.  We verify the
    // returned path has the expected suffix.
    let result = ConfigManager::with_default_root_dir();
    assert!(
        result.is_ok(),
        "with_default_root_dir() should succeed when HOME is set"
    );
    let manager = result.unwrap();
    let config_dir = manager.config_dir().to_path_buf();
    assert!(
        config_dir.ends_with(".closeclaw/config"),
        "config_dir should end with .closeclaw/config, got: {}",
        config_dir.display()
    );
}

// ---------------------------------------------------------------------------
// Config directory exists after call
// ---------------------------------------------------------------------------

/// Test: with_default_root_dir() ensures the config directory exists on disk.
#[test]
fn test_with_default_root_dir_creates_config_dir() {
    let manager = ConfigManager::with_default_root_dir().unwrap();
    let config_dir = manager.config_dir().to_path_buf();
    assert!(
        config_dir.is_dir(),
        "config directory should exist on disk: {}",
        config_dir.display()
    );
}

// ---------------------------------------------------------------------------
// Idempotency — calling twice returns the same path
// ---------------------------------------------------------------------------

/// Test: with_default_root_dir() is idempotent — two sequential calls
/// return the same config_dir path.
#[test]
fn test_with_default_root_dir_idempotent() {
    let first = ConfigManager::with_default_root_dir().unwrap();
    let second = ConfigManager::with_default_root_dir().unwrap();
    assert_eq!(
        first.config_dir(),
        second.config_dir(),
        "two calls should produce the same config_dir"
    );
}
