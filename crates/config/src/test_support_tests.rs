//! Self-tests for the shared test-only path helpers (`test_support`).

use crate::test_support::ensure_dir;
use tempfile::TempDir;

/// Normal path: `ensure_dir` materializes the directory (with any
/// missing parents) and returns the same path, which exists as a dir.
#[test]
fn test_ensure_dir_creates_directory() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("config_dir");

    let returned = ensure_dir(&path);

    assert_eq!(returned, path, "returned path must be the requested one");
    assert!(path.exists(), "directory must exist after ensure_dir");
    assert!(path.is_dir(), "ensured path must be a directory");
}

/// Idempotence: calling `ensure_dir` on an already-existing directory
/// succeeds without error and keeps it a directory.
#[test]
fn test_ensure_dir_is_idempotent_for_existing_directory() {
    let tmp = TempDir::new().unwrap();
    let path = ensure_dir(&tmp.path().join("already_there"));

    let returned = ensure_dir(&path);

    assert_eq!(returned, path);
    assert!(path.is_dir(), "directory must remain a directory");
}
