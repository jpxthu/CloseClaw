//! Configuration directory resolution.
//!
//! Returns the platform-appropriate root and config directories for CloseClaw.
//! - Root: `~/.closeclaw` (PID files, agents/, templates/, skills/, etc.)
//! - Config: `~/.closeclaw/config` (JSON config files: models.json, channels.json, etc.)
//!
//! Windows equivalents: `%APPDATA%\closeclaw` and `%APPDATA%\closeclaw\config`.
//!
//! Both [`root_dir`] and [`config_dir`] guarantee the returned directory exists
//! on disk (created via `create_dir_all`), so callers never need to create them.

use std::path::PathBuf;

use anyhow::Context;

/// Computes the root CloseClaw path under the given home directory.
///
/// Does **not** create the directory — pure path computation only.
/// Used by [`root_dir`] and indirectly available for testing.
#[cfg(unix)]
pub(crate) fn root_dir_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(".closeclaw")
}

#[cfg(not(unix))]
pub(crate) fn root_dir_path(appdata: &str) -> PathBuf {
    PathBuf::from(appdata).join("closeclaw")
}

/// Computes the config path under the given home directory.
///
/// Does **not** create the directory — pure path computation only.
/// Used by [`config_dir`] and indirectly available for testing.
#[cfg(unix)]
pub(crate) fn config_dir_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(".closeclaw").join("config")
}

#[cfg(not(unix))]
pub(crate) fn config_dir_path(appdata: &str) -> PathBuf {
    PathBuf::from(appdata).join("closeclaw").join("config")
}

/// Returns the **root** CloseClaw directory for the current platform,
/// creating it (and any parent directories) if it does not yet exist.
///
/// This is the top-level directory that contains the `config/` subdirectory,
/// `agents/`, `templates/`, `skills/`, PID files, and the admin socket.
///
/// - Linux/macOS: `~/.closeclaw`
/// - Windows: `%APPDATA%\closeclaw`
///
/// The directory is created idempotently — calling this function when the
/// directory already exists is safe and has no side effects.
///
/// # Errors
///
/// Returns an error if the home directory / APPDATA cannot be determined,
/// or if the directory cannot be created (e.g. a parent path is a file
/// instead of a directory).
pub fn root_dir() -> anyhow::Result<PathBuf> {
    #[cfg(unix)]
    {
        let home = std::env::var("HOME")
            .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
        let path = root_dir_path(&home);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create root dir at {}", path.display()))?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not set"))?;
        let path = root_dir_path(&appdata);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create root dir at {}", path.display()))?;
        Ok(path)
    }
}

/// Returns the **config** directory for the current platform,
/// creating it (and any parent directories) if it does not yet exist.
///
/// This is the subdirectory that contains JSON config files (models.json,
/// channels.json, gateway.json, plugins.json, system.json).
///
/// - Linux/macOS: `~/.closeclaw/config`
/// - Windows: `%APPDATA%\closeclaw\config`
///
/// The directory is created idempotently — calling this function when the
/// directory already exists is safe and has no side effects.
///
/// # Errors
///
/// Returns an error if the home directory / APPDATA cannot be determined,
/// or if the directory cannot be created (e.g. a parent path is a file
/// instead of a directory).
pub fn config_dir() -> anyhow::Result<PathBuf> {
    #[cfg(unix)]
    {
        let home = std::env::var("HOME")
            .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
        let path = config_dir_path(&home);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create config dir at {}", path.display()))?;
        Ok(path)
    }
    #[cfg(not(unix))]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not set"))?;
        let path = config_dir_path(&appdata);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("failed to create config dir at {}", path.display()))?;
        Ok(path)
    }
}
