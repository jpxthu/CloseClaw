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

// ── Path computation (platform-specific env var, suffix) ──────────

/// Returns the env var name used to locate the user's base directory.
#[cfg(unix)]
fn home_env_var_name() -> &'static str {
    "HOME"
}

#[cfg(not(unix))]
fn home_env_var_name() -> &'static str {
    "APPDATA"
}

/// The platform-specific directory name under the user's home.
#[cfg(unix)]
const DIR_NAME: &str = ".closeclaw";

#[cfg(not(unix))]
const DIR_NAME: &str = "closeclaw";

/// Computes the root CloseClaw path under the given home directory.
///
/// Does **not** create the directory — pure path computation only.
pub(crate) fn root_dir_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(DIR_NAME)
}

/// Computes the config path under the given home directory.
///
/// Does **not** create the directory — pure path computation only.
pub(crate) fn config_dir_path(home: &str) -> PathBuf {
    PathBuf::from(home).join(DIR_NAME).join("config")
}

// ── Inner (injectable) resolution helpers ─────────────────────────

/// Returns the root directory under `home`, creating it on disk.
///
/// Shared by [`root_dir`]; exists separately so tests can inject a
/// synthetic `home` value without touching environment variables.
pub(crate) fn root_dir_inner(home: &str) -> anyhow::Result<PathBuf> {
    let path = root_dir_path(home);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("failed to create root dir at {}", path.display()))?;
    Ok(path)
}

/// Returns the config directory under `home`, creating it on disk.
///
/// Shared by [`config_dir`]; exists separately so tests can inject a
/// synthetic `home` value without touching environment variables.
pub(crate) fn config_dir_inner(home: &str) -> anyhow::Result<PathBuf> {
    let path = config_dir_path(home);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("failed to create config dir at {}", path.display()))?;
    Ok(path)
}

// ── Public API ────────────────────────────────────────────────────

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
    let home = std::env::var(home_env_var_name())
        .map_err(|_| anyhow::anyhow!("{} environment variable not set", home_env_var_name()))?;
    root_dir_inner(&home)
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
    let home = std::env::var(home_env_var_name())
        .map_err(|_| anyhow::anyhow!("{} environment variable not set", home_env_var_name()))?;
    config_dir_inner(&home)
}
