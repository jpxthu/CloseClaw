//! File path normalization and permissions.
//!
//! Provides utilities to normalize path separators to `/`, expand
//! environment-variable-based path prefixes (e.g. `~`), and check or
//! modify file permissions across platforms.

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
/// Also expands `%VAR%`-style environment variable prefixes. Common Windows
/// variables include `%APPDATA%`, `%LOCALAPPDATA%`, `%USERPROFILE%`, etc.
/// If the environment variable is not set, the original path is returned
/// unchanged.
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use closeclaw_platform::fs::expand_home;
/// // `~` expands to $HOME
/// // expand_home(Path::new("~/foo"));
///
/// // `%APPDATA%` expands to $APPDATA
/// // expand_home(Path::new("%APPDATA%/foo"));
/// ```
pub fn expand_home(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if let Some(rest) = s.strip_prefix("%") {
        if let Some(var_end) = rest.find('%') {
            let var_name = &rest[..var_end];
            if !var_name.is_empty() {
                if let Ok(val) = std::env::var(var_name) {
                    return PathBuf::from(val).join(&rest[var_end + 1..]);
                }
            }
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        mode & 0o400 != 0 // User read bit
    }
    #[cfg(not(unix))]
    {
        !perms.readonly()
    }
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        mode & 0o200 != 0 // User write bit
    }
    #[cfg(not(unix))]
    {
        !perms.readonly()
    }
}

/// Checks whether a file has the executable permission (Unix) or is
/// not read-only (Windows fallback).
///
/// On Unix, returns `true` if the user-execute bit is set. On Windows,
/// returns `true` if the file is not read-only (since Windows does not
/// have a distinct executable permission).
pub fn check_executable(path: &Path) -> bool {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let perms = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        mode & 0o100 != 0 // User execute bit
    }
    #[cfg(not(unix))]
    {
        // Windows: no distinct executable permission; treat as not-readonly
        !perms.readonly()
    }
}

/// Sets the executable permission on a file (Unix) or clears the
/// read-only flag (Windows).
///
/// On Unix, toggles the user-execute bit. On Windows, sets or clears
/// the read-only attribute as a best-effort equivalent.
///
/// Returns an error if the file does not exist or the operation fails.
pub fn set_executable(path: &Path, executable: bool) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut perms = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = perms.mode();
        let new_mode = if executable {
            mode | 0o100
        } else {
            mode & !0o100
        };
        perms.set_mode(new_mode);
    }
    #[cfg(not(unix))]
    {
        perms.set_readonly(!executable);
    }
    std::fs::set_permissions(path, perms)?;
    Ok(())
}
