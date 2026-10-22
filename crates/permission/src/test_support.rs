//! Shared test-only helpers for temporary-path construction.

/// Build a system-temp-dir path `name` as a `String` for test inputs.
pub(crate) fn tmp_str(name: &str) -> String {
    std::env::temp_dir()
        .join(name)
        .to_string_lossy()
        .into_owned()
}
