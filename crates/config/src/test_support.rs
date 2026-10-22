//! Shared test-only helpers for temporary subdirectory construction.

/// Create subdirectory `rel` (any relative subpath, possibly nested,
/// e.g. `config.json` to express "config.json as a directory") under an
/// existing `base`, returning the created directory path.
pub(crate) fn create_subdir(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let sub = base.join(rel);
    std::fs::create_dir_all(&sub).unwrap();
    sub
}

/// Remove directory `path` recursively.
///
/// Scenario-simulation helper only: reproduces on-disk states such as
/// "backup dir absent", "config dir vanishes" or "backup dir replaced by
/// a file" that the production failure paths react to. Callers decide
/// error handling — typically `.ok()` since the directory may not exist.
pub(crate) fn remove_subdir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}
