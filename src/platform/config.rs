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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_dir_returns_valid_path() {
        let dir = config_dir().unwrap();
        assert!(dir.is_absolute(), "config_dir must return an absolute path");

        let path_str = dir.to_string_lossy();
        assert!(
            path_str.contains("closeclaw"),
            "config_dir must contain 'closeclaw': {path_str}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_config_dir_unix_format() {
        let dir = config_dir().unwrap();
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.ends_with("/.closeclaw"),
            "Unix config_dir must end with /.closeclaw: {path_str}"
        );
    }
}
