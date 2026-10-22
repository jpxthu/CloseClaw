//! Shared test-only helpers for temporary-path construction.

/// Build a system-temp-dir path `name` for test inputs.
pub(crate) fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(name)
}
