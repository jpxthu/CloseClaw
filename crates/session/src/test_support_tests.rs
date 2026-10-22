//! Self-tests for the shared test-only path helpers (`test_support`).

use crate::test_support::create_subdir;
use tempfile::TempDir;

/// Normal path: `create_subdir` materializes `base.join(rel)` and returns
/// exactly that path, existing as a directory on disk.
#[test]
fn test_create_subdir_creates_dir_and_returns_existing_dir_path() {
    let tmp = TempDir::new().unwrap();

    let dir = create_subdir(tmp.path(), "plans");

    assert_eq!(
        dir,
        tmp.path().join("plans"),
        "returned path must be base.join(rel)"
    );
    assert!(dir.is_dir(), "returned path must exist as a directory");
}

/// Multi-level relative subpath: a nested `plans/archive` form creates the
/// leaf directory including intermediate parents, rooted under base.
#[test]
fn test_create_subdir_creates_nested_rel_path_with_parents() {
    let tmp = TempDir::new().unwrap();

    let dir = create_subdir(tmp.path(), "plans/archive");

    assert_eq!(
        dir,
        tmp.path().join("plans").join("archive"),
        "nested rel must resolve under base"
    );
    assert!(
        dir.is_dir(),
        "nested subdir must exist as a directory (parents created)"
    );
}

/// Boundary: re-creating an already-existing directory is idempotent —
/// the helper neither errors nor wipes pre-existing content.
#[test]
fn test_create_subdir_is_idempotent_for_existing_directory() {
    let tmp = TempDir::new().unwrap();
    let first = create_subdir(tmp.path(), "plans");
    let marker = first.join("marker.txt");
    std::fs::write(&marker, "ok").unwrap();

    let second = create_subdir(tmp.path(), "plans");

    assert_eq!(second, first, "same rel must resolve to the same path");
    assert!(
        second.is_dir(),
        "directory must still exist after re-creation"
    );
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "ok",
        "pre-existing content must survive the idempotent re-creation"
    );
}
