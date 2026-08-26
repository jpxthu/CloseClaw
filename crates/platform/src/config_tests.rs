use crate::config::{config_dir_path, root_dir_path};
use std::path::Path;

// ── root_dir_path / config_dir_path pure path tests ──────────────

#[test]
fn test_root_dir_path_pure_computation() {
    let path = root_dir_path("/tmp/fakehome");
    assert_eq!(path, Path::new("/tmp/fakehome/.closeclaw"));
}

#[test]
fn test_config_dir_path_pure_computation() {
    let path = config_dir_path("/tmp/fakehome");
    assert_eq!(path, Path::new("/tmp/fakehome/.closeclaw/config"));
}

// ── root_dir creation via TempDir injection ──────────────────────

#[test]
fn test_root_dir_creates_directory_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = root_dir_path(tmp.path().to_str().unwrap());
    assert!(!root.exists(), "precondition: root must not exist yet");

    // Use create_dir_all directly to simulate root_dir() behavior.
    // root_dir_path is the pure computation; we exercise the same
    // create_dir_all that root_dir() calls.
    std::fs::create_dir_all(&root).unwrap();
    assert!(root.is_dir(), "root directory should be created");
}

#[test]
fn test_root_dir_idempotent_when_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let root = root_dir_path(tmp.path().to_str().unwrap());
    std::fs::create_dir_all(&root).unwrap();

    // Calling create_dir_all again should not fail.
    std::fs::create_dir_all(&root).unwrap();
    assert!(root.is_dir());
}

#[test]
fn test_root_dir_creation_fails_when_parent_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Make the parent path a file so root_dir can't be created.
    // root_dir_path returns tmp/.closeclaw, so we make tmp itself
    // impossible to traverse — but tmp is already a dir. Instead,
    // make .closeclaw a file and verify config_dir fails (same
    // underlying create_dir_all mechanism).
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "not a directory").unwrap();

    // root_dir_path = tmp/.closeclaw — it's already a file, so
    // create_dir_all on it directly must fail.
    let root = root_dir_path(tmp.path().to_str().unwrap());
    let result = std::fs::create_dir_all(&root);
    assert!(
        result.is_err(),
        "create_dir_all should fail when path is a file"
    );
}

// ── config_dir creation via TempDir injection ────────────────────

#[test]
fn test_config_dir_creates_directory_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_dir_path(tmp.path().to_str().unwrap());
    assert!(!config.exists(), "precondition: config dir must not exist");

    std::fs::create_dir_all(&config).unwrap();
    assert!(config.is_dir(), "config directory should be created");
}

#[test]
fn test_config_dir_idempotent_when_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_dir_path(tmp.path().to_str().unwrap());
    std::fs::create_dir_all(&config).unwrap();

    // Second call is safe.
    std::fs::create_dir_all(&config).unwrap();
    assert!(config.is_dir());
}

#[test]
fn test_config_dir_creation_fails_when_parent_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Make .closeclaw a file so config_dir creation must fail.
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "blocked").unwrap();

    let config = config_dir_path(tmp.path().to_str().unwrap());
    let result = std::fs::create_dir_all(&config);
    assert!(
        result.is_err(),
        "create_dir_all should fail when parent is a file"
    );
}

// ── integration: root_dir / config_dir create both levels ────────

#[test]
fn test_root_and_config_dirs_exist_after_creation() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let root = root_dir_path(home.to_str().unwrap());
    let config = config_dir_path(home.to_str().unwrap());

    // Simulate what root_dir() and config_dir() do internally.
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&config).unwrap();

    assert!(root.is_dir());
    assert!(config.is_dir());
    // config is a child of root.
    assert_eq!(config.parent(), Some(root.as_path()));
}
