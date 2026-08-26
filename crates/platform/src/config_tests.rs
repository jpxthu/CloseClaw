use crate::config::{config_dir_inner, config_dir_path, root_dir_inner, root_dir_path};
use std::path::Path;

// ── Pure path computation tests ──────────────────────────────────

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

// ── root_dir_inner: injectable tests ─────────────────────────────

#[test]
fn test_root_dir_inner_creates_directory_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let result = root_dir_inner(tmp.path().to_str().unwrap()).unwrap();
    assert!(
        result.is_dir(),
        "root_dir_inner should create the directory"
    );
    assert_eq!(result, root_dir_path(tmp.path().to_str().unwrap()));
}

#[test]
fn test_root_dir_inner_idempotent_when_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let first = root_dir_inner(home).unwrap();
    let second = root_dir_inner(home).unwrap();
    assert_eq!(first, second);
    assert!(first.is_dir());
}

#[test]
fn test_root_dir_inner_fails_when_parent_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Place a regular file at the path where root_dir would create a directory.
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "not a directory").unwrap();

    let result = root_dir_inner(tmp.path().to_str().unwrap());
    assert!(result.is_err(), "must fail when path is already a file");
}

#[test]
fn test_root_dir_inner_error_has_context() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "blocked").unwrap();

    let err = root_dir_inner(tmp.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed to create root dir"),
        "error must include context message, got: {msg}"
    );
}

// ── config_dir_inner: injectable tests ───────────────────────────

#[test]
fn test_config_dir_inner_creates_directory_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let result = config_dir_inner(tmp.path().to_str().unwrap()).unwrap();
    assert!(
        result.is_dir(),
        "config_dir_inner should create the directory"
    );
    assert_eq!(result, config_dir_path(tmp.path().to_str().unwrap()));
}

#[test]
fn test_config_dir_inner_idempotent_when_already_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let first = config_dir_inner(home).unwrap();
    let second = config_dir_inner(home).unwrap();
    assert_eq!(first, second);
    assert!(first.is_dir());
}

#[test]
fn test_config_dir_inner_fails_when_parent_is_file() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "blocked").unwrap();

    let result = config_dir_inner(tmp.path().to_str().unwrap());
    assert!(result.is_err(), "must fail when path is already a file");
}

#[test]
fn test_config_dir_inner_error_has_context() {
    let tmp = tempfile::tempdir().unwrap();
    let blocker = tmp.path().join(".closeclaw");
    std::fs::write(&blocker, "blocked").unwrap();

    let err = config_dir_inner(tmp.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("failed to create config dir"),
        "error must include context message, got: {msg}"
    );
}

// ── Integration: both root and config exist after creation ───────

#[test]
fn test_root_and_config_dirs_exist_after_creation() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();
    let root = root_dir_inner(home).unwrap();
    let config = config_dir_inner(home).unwrap();

    assert!(root.is_dir());
    assert!(config.is_dir());
    // config is a child of root.
    assert_eq!(config.parent(), Some(root.as_path()));
}
