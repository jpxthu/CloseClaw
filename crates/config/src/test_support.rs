//! Shared test-only helpers for temporary subdirectory construction.

/// Create subdirectory `rel` (any relative subpath, possibly nested,
/// e.g. `config.json` to express "config.json as a directory") under an
/// existing `base`, returning the created directory path.
pub(crate) fn create_subdir(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let sub = base.join(rel);
    std::fs::create_dir_all(&sub).unwrap();
    sub
}
