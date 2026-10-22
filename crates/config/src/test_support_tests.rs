//! Self-tests for the shared test-only path helpers (`test_support`).

use crate::test_support::{create_subdir, ensure_dir, remove_subdir};
use tempfile::TempDir;

/// Normal path: `remove_subdir` deletes the directory recursively —
/// nested content included — and reports success.
#[test]
fn test_remove_subdir_deletes_directory_recursively() {
    let tmp = TempDir::new().unwrap();
    let dir = create_subdir(tmp.path(), "agents/alpha");
    let marker = dir.join("marker.txt");
    std::fs::write(&marker, "ok").unwrap();

    let result = remove_subdir(&dir);

    assert!(result.is_ok(), "removal of an existing dir must succeed");
    assert!(!dir.exists(), "directory must be gone after removal");
}

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

/// Error path: removing a directory that does not exist surfaces the
/// underlying error instead of panicking — callers decide handling
/// (typically `.ok()` per the helper contract).
#[test]
fn test_remove_subdir_returns_error_for_missing_directory() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("never_created");

    let result = remove_subdir(&missing);

    assert!(
        result.is_err(),
        "removal of a missing dir must surface the io error"
    );
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::NotFound,
        "missing-dir removal must report NotFound"
    );
}
