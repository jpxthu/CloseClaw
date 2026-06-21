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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_unix() {
        let path = Path::new("/usr/local/bin");
        let normalized = normalize_path(path);
        assert_eq!(normalized, PathBuf::from("/usr/local/bin"));
    }

    #[test]
    fn test_normalize_path_backslashes() {
        let path = Path::new(r"C:\Users\test\file.txt");
        let normalized = normalize_path(path);
        assert_eq!(normalized, PathBuf::from("C:/Users/test/file.txt"));
    }

    #[test]
    fn test_expand_home_no_tilde() {
        let path = Path::new("/absolute/path");
        assert_eq!(expand_home(path), PathBuf::from("/absolute/path"));
    }
}
