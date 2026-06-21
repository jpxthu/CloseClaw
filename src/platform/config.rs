//! Configuration directory resolution.
//!
//! Returns the platform-appropriate root directory for CloseClaw configuration files.
//! Linux/macOS: `~/.closeclaw`
//! Windows: `%APPDATA%\closeclaw`

use std::path::PathBuf;

/// Returns the configuration directory path for the current platform.
///
/// # Errors
///
/// Returns an error if the home directory or APPDATA cannot be determined.
pub fn config_dir() -> anyhow::Result<PathBuf> {
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
