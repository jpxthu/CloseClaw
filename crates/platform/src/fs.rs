//! File path normalization.
//!
//! Provides utilities to normalize path separators to `/` and expand
//! environment-variable-based path prefixes (e.g. `~`).

use std::path::{Path, PathBuf};

/// Normalizes a path to use `/` as the separator.
///
/// This is useful for canonicalizing paths across platforms before
/// comparing or storing them.
pub fn normalize_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy().replace('\\', "/");
    PathBuf::from(s)
}

/// Expands `~` at the start of a path to the user's home directory.
///
/// On Windows, also expands `%APPDATA%`.
pub fn expand_home(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}
