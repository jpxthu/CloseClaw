//! Shared test-only helpers for temporary subdirectory construction.

/// Create subdirectory `rel` (any relative subpath, possibly nested,
/// e.g. `config.json` to express "config.json as a directory") under
/// `base`, returning the created directory path.
///
/// Mirrors `create_dir_all` semantics: missing path components —
/// including `base` itself — are created, not required to pre-exist.
pub(crate) fn create_subdir(base: &std::path::Path, rel: &str) -> std::path::PathBuf {
    let sub = base.join(rel);
    std::fs::create_dir_all(&sub).unwrap();
    sub
}

/// Ensure `path` exists as a directory, creating it (with any missing
/// parents) if needed, and return the path.
///
/// For callers that need the base directory itself materialized —
/// instead of the empty-`rel` edge usage `create_subdir(base, "")`.
pub(crate) fn ensure_dir(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(path).unwrap();
    path.to_path_buf()
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
