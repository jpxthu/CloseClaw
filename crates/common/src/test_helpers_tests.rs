//! Unit tests for `test_helpers::ensure_dir`'s TempDir contract.

use crate::test_helpers::ensure_dir;

#[test]
fn test_ensure_dir_creates_subdir_under_tempdir() {
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let subdir = tmp.path().join("nested/deeper");
    ensure_dir(&subdir).expect("create nested subdir under TempDir");
    assert!(subdir.is_dir(), "subdir should exist: {subdir:?}");
}

// The panic path under test comes from `debug_assert!` in `ensure_dir`,
// which is compiled out in release builds — this counter-example is only
// meaningful for the debug profile, so it is compiled (and run) solely
// when `debug_assertions` is on.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "ensure_dir")]
fn test_ensure_dir_panics_for_path_outside_tempdir() {
    let outside = std::env::temp_dir()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("/proc"));
    ensure_dir(&outside.join("closeclaw_ensure_dir_outside_probe"))
        .expect("ensure_dir: create_dir_all outside temp dir must fail");
}
