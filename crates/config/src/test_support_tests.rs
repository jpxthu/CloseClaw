//! Self-tests for the shared test-only path helpers (`test_support`).

use crate::test_support::{create_subdir, remove_subdir};
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
