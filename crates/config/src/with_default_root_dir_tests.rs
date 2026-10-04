//! Tests for the `ConfigManager::with_default_root_dir()` composition.
//!
//! `with_default_root_dir()` resolves the real `HOME` via
//! `closeclaw_platform::config::root_dir()`, which tests must not touch.
//! These tests replicate its composition — `root_dir()` →
//! `join("config")` → `ConfigManager::new` — using the injectable
//! `closeclaw_platform::config::root_dir_inner(home)` seam with a
//! `TempDir` home. The real-HOME resolution step itself is covered by the
//! `closeclaw_platform::config_tests` root_dir_inner tests.

use super::*;

// ---------------------------------------------------------------------------
// Happy path — injected home → returns correct config directory
// ---------------------------------------------------------------------------

/// Test: the with_default_root_dir() composition succeeds for an injected
/// home, producing a config_dir that ends with `/.closeclaw/config`.
#[test]
fn test_with_default_root_dir_success() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let root = closeclaw_platform::config::root_dir_inner(home).unwrap();
    let manager = ConfigManager::new(root.join("config")).unwrap();
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
/// directory exists on disk (root created by `root_dir_inner`, config
/// bootstrapped by `ConfigManager::new` via its `.backups` dir).
#[test]
fn test_with_default_root_dir_creates_config_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let root = closeclaw_platform::config::root_dir_inner(home).unwrap();
    let manager = ConfigManager::new(root.join("config")).unwrap();
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
/// sequential root_dir_inner() calls return the same path, so both
/// managers resolve the same config_dir.
#[test]
fn test_with_default_root_dir_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let first = closeclaw_platform::config::root_dir_inner(home).unwrap();
    let second = closeclaw_platform::config::root_dir_inner(home).unwrap();
    assert_eq!(first, second, "two calls should produce the same root dir");
    let first_manager = ConfigManager::new(first.join("config")).unwrap();
    let second_manager = ConfigManager::new(second.join("config")).unwrap();
    assert_eq!(
        first_manager.config_dir(),
        second_manager.config_dir(),
        "two calls should produce the same config_dir"
    );
}
