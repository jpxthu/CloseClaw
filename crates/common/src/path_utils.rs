//! Path utilities shared across crates.

use std::path::{Path, PathBuf};

/// Canonicalize `path` if possible; fall back to the original path.
///
/// This resolves symlinks and normalizes `.` / `..` segments.
/// If canonicalization fails (e.g. the file does not exist yet),
/// the original path is returned as-is.
pub fn canonicalize_or_clone(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "path_utils_tests.rs"]
mod tests;
