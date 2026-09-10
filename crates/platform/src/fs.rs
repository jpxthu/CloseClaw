//! File path normalization and permissions.
//!
//! Provides utilities to normalize path separators to `/`, expand
//! the `~` home directory prefix, and check or modify file permissions.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// Normalizes a path to use `/` as the separator.
///
/// This is useful for canonicalizing paths across platforms before
/// comparing or storing them.
pub fn normalize_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy().replace('\\', "/");
    PathBuf::from(s)
}

/// Converts an internal `/`-separated path to the current platform's
/// native path representation.
///
/// On platforms that use `/` as the path separator (Linux, macOS,
/// WSL2), this is equivalent to [`normalize_path`].
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use closeclaw_platform::fs::to_platform_path;
/// let p = to_platform_path(Path::new("foo/bar"));
/// assert_eq!(p, std::path::PathBuf::from("foo/bar"));
/// ```
pub fn to_platform_path(path: &Path) -> PathBuf {
    normalize_path(path)
}

/// Expands `~` at the start of a path to the user's home directory.
///
/// Supports `~`, `~/`, and `~/rest`. A bare `~otheruser` prefix is
/// left unchanged (not a home shorthand for the current user).
///
/// Uses [`dirs::home_dir`] which falls back to `getpwuid` when `HOME`
/// is unset, returning `None` only if both sources fail.
///
/// # Examples
///
/// ```no_run
/// # use std::path::Path;
/// # use closeclaw_platform::fs::expand_home;
/// // `~` expands to an absolute home directory path
/// assert!(expand_home(Path::new("~")).is_absolute());
/// ```
pub fn expand_home(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    // bare `~` → home
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
        return path.to_path_buf();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

/// Expands environment variable references (`$VAR` / `${VAR}`) in a
/// path to their values.
///
/// Undefined variables are preserved as their literal text (no error,
/// no empty replacement). This keeps the path diagnosable and avoids
/// side effects from reading process-global env.
///
/// Windows-style `%VAR%` is intentionally not supported.
///
/// # Examples
///
/// ```
/// # use std::path::{Path, PathBuf};
/// # use closeclaw_platform::fs::expand_env;
/// // $VAR and ${VAR} are expanded; undefined vars stay literal
/// let p = expand_env(Path::new("$NONEXISTENT_XYZ"));
/// assert_eq!(p, PathBuf::from("$NONEXISTENT_XYZ"));
/// ```
pub fn expand_env(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if !s.contains('$') {
        return path.to_path_buf();
    }
    // Match ${VAR} or $VAR (POSIX variable syntax).
    // ${VAR} uses a non-greedy capture up to the closing brace.
    // $VAR uses a capture of valid identifier characters.
    static ENV_VAR_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\$\{([^}]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)").unwrap());
    let re = &*ENV_VAR_RE;
    let mut result = String::with_capacity(s.len());
    let mut last = 0;
    for m in re.find_iter(&s) {
        result.push_str(&s[last..m.start()]);
        // Extract the variable name from the match
        let var_name = if m.as_str().starts_with("${") {
            &m.as_str()[2..m.as_str().len() - 1] // strip ${ and }
        } else {
            &m.as_str()[1..] // strip $
        };
        if var_name.is_empty() {
            // Empty var name (e.g. `${}`) → preserve the literal `$`
            result.push('$');
            if m.as_str().starts_with("${") {
                result.push_str("{}")
            }
        } else {
            match std::env::var(var_name) {
                Ok(val) => result.push_str(&val),
                Err(_) => {
                    // Undefined variable → preserve the literal text
                    result.push_str(m.as_str());
                }
            }
        }
        last = m.end();
    }
    result.push_str(&s[last..]);
    PathBuf::from(result)
}

/// Complete path expansion: home shorthand (`~`) → environment
/// variables (`$VAR`/`${VAR}`) → normalized `/` separators.
///
/// This is the single entry point for all path "abbreviation expansion"
/// as required by the design doc. It chains [`expand_home`],
/// [`expand_env`], and [`normalize_path`].
///
/// # Examples
///
/// ```
/// # use std::path::Path;
/// # use closeclaw_platform::fs::expand_path;
/// let p = expand_path(Path::new("~/foo"));
/// assert!(p.to_string_lossy().contains("/foo"));
/// ```
pub fn expand_path(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    let expanded = expand_env(&expanded);
    normalize_path(&expanded)
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
