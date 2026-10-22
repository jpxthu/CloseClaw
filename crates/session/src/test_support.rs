//! Shared test-only helpers for temporary-path and subdirectory construction.

/// Build a system-temp-dir path `name` for test inputs.
pub(crate) fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}

/// Create subdirectory `rel` (any relative subpath, possibly nested,
/// e.g. `plans/archive`) under an existing `base`, returning the created
/// directory path.
pub(crate) fn create_subdir(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let sub = base.join(rel);
    std::fs::create_dir_all(&sub).unwrap();
    sub
}
