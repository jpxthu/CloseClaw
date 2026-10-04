//! Tests for the `ConfigManager::with_default_root_dir()` composition.
//!
//! `with_default_root_dir()` resolves the real `HOME` via
//! `closeclaw_platform::config::root_dir()`, which tests must not touch.
//! They therefore call the `#[cfg(test)]`
//! `ConfigManager::with_default_root_dir_home(home)` seam, which resolves
//! the root under an injected TempDir home through the injectable
//! `closeclaw_platform::config::root_dir_inner` and then runs the same
//! shared `default_root_at` composition (`join("config")` →
//! `ConfigManager::new`) as production. The real-HOME resolution step
//! itself is covered by the `closeclaw_platform::config_tests`
//! root_dir_inner tests.

use super::*;

// ---------------------------------------------------------------------------
// Happy path — injected home → returns correct config directory
// ---------------------------------------------------------------------------

/// Test: the with_default_root_dir() composition succeeds for an injected
/// home, producing a config_dir that ends with `/.closeclaw/config`.
#[test]
fn test_with_default_root_dir_success() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = ConfigManager::with_default_root_dir_home(tmp.path().to_str().unwrap()).unwrap();
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

/// Test: the with_default_root_dir() composition ensures the config
/// directory exists on disk (root created by root resolution, config
/// bootstrapped by `ConfigManager::new` via its `.backups` dir).
#[test]
fn test_with_default_root_dir_creates_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = ConfigManager::with_default_root_dir_home(tmp.path().to_str().unwrap()).unwrap();
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

/// Test: the with_default_root_dir() composition is idempotent — two
/// calls with the same injected home resolve the same root, so both
/// managers share the same config_dir.
#[test]
fn test_with_default_root_dir_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let first_manager = ConfigManager::with_default_root_dir_home(home).unwrap();
    let second_manager = ConfigManager::with_default_root_dir_home(home).unwrap();
    assert_eq!(
        first_manager.config_dir(),
        second_manager.config_dir(),
        "two calls should produce the same config_dir"
    );
}
