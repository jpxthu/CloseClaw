//! File path normalization and permissions.
//!
//! Provides utilities to normalize path separators to `/`, expand
//! the `~` home directory prefix, and check or modify file permissions.

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
/// If `HOME` is not set, the original path is returned unchanged.
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use closeclaw_platform::fs::expand_home;
/// // `~` expands to $HOME
/// // expand_home(Path::new("~/foo"));
/// ```
pub fn expand_home(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

/// Checks whether a file or directory is readable.
///
/// Returns `true` if the path exists and has read permission for the
/// current user, `false` otherwise.
pub fn check_readable(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let perms = metadata.permissions();
    use std::os::unix::fs::PermissionsExt;
    let mode = perms.mode();
    mode & 0o400 != 0 // User read bit
}

/// Checks whether a file or directory is writable.
///
/// Returns `true` if the path exists and has write permission for the
/// current user, `false` otherwise.
pub fn check_writable(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let perms = metadata.permissions();
    use std::os::unix::fs::PermissionsExt;
    let mode = perms.mode();
    mode & 0o200 != 0 // User write bit
}

/// Checks whether a file has the executable permission.
///
/// Returns `true` if the user-execute bit is set.
pub fn check_executable(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let perms = metadata.permissions();
    use std::os::unix::fs::PermissionsExt;
    let mode = perms.mode();
    mode & 0o100 != 0 // User execute bit
}

/// Sets the executable permission on a file.
///
/// Toggles the user-execute bit.
///
/// Returns an error if the file does not exist or the operation fails.
pub fn set_executable(path: &Path, executable: bool) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut perms = metadata.permissions();
    use std::os::unix::fs::PermissionsExt;
    let mode = perms.mode();
    let new_mode = if executable {
        mode | 0o100
    } else {
        mode & !0o100
    };
    perms.set_mode(new_mode);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}
