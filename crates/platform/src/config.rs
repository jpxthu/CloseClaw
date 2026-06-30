//! Configuration directory resolution.
//!
//! Returns the platform-appropriate root and config directories for CloseClaw.
//! - Root: `~/.closeclaw` (PID files, agents/, templates/, skills/, etc.)
//! - Config: `~/.closeclaw/config` (JSON config files: models.json, channels.json, etc.)
//!
//! Windows equivalents: `%APPDATA%\closeclaw` and `%APPDATA%\closeclaw\config`.

use std::path::PathBuf;

/// Returns the **root** CloseClaw directory for the current platform.
///
/// This is the top-level directory that contains the `config/` subdirectory,
/// `agents/`, `templates/`, `skills/`, PID files, and the admin socket.
///
/// - Linux/macOS: `~/.closeclaw`
/// - Windows: `%APPDATA%\closeclaw`
///
/// # Errors
///
/// Returns an error if the home directory or APPDATA cannot be determined.
pub fn root_dir() -> anyhow::Result<PathBuf> {
    #[cfg(unix)]
    {
        let home = std::env::var("HOME")
            .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
        Ok(PathBuf::from(home).join(".closeclaw"))
    }
    #[cfg(not(unix))]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not set"))?;
        Ok(PathBuf::from(appdata).join("closeclaw"))
    }
}

/// Returns the **config** directory for the current platform.
///
/// This is the subdirectory that contains JSON config files (models.json,
/// channels.json, gateway.json, plugins.json, system.json).
///
/// - Linux/macOS: `~/.closeclaw/config`
/// - Windows: `%APPDATA%\closeclaw\config`
///
/// # Errors
///
/// Returns an error if the home directory or APPDATA cannot be determined.
pub fn config_dir() -> anyhow::Result<PathBuf> {
    #[cfg(unix)]
    {
        let home = std::env::var("HOME")
            .map_err(|_| anyhow::anyhow!("HOME environment variable not set"))?;
        Ok(PathBuf::from(home).join(".closeclaw").join("config"))
    }
    #[cfg(not(unix))]
    {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| anyhow::anyhow!("APPDATA environment variable not set"))?;
        Ok(PathBuf::from(appdata).join("closeclaw").join("config"))
    }
}
