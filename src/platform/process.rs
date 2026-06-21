//! Process lifecycle management.
//!
//! Provides PID file read/write and signal-based process termination.
//! Unix uses SIGTERM/SIGINT; Windows uses process termination API.

use std::path::{Path, PathBuf};
use tracing::info;

/// Returns the platform-specific PID file path.
///
/// On Unix: `{config_dir}/daemon.pid`
/// On Windows: `{config_dir}\daemon.pid`
pub fn pid_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("daemon.pid")
}

/// Writes the given PID to the specified file, creating or overwriting it.
pub fn write_pid_file(path: &Path, pid: u32) -> anyhow::Result<()> {
    std::fs::write(path, pid.to_string())?;
    Ok(())
}

/// Reads a PID from the specified file.
///
/// Returns `None` if the file does not exist or cannot be parsed.
pub fn read_pid_file(path: &Path) -> Option<u32> {
    let content = std::fs::read_to_string(path).ok()?;
    content.trim().parse::<u32>().ok()
}

/// Sends a termination signal to the process identified by `pid`.
///
/// On Unix, sends SIGTERM. On Windows, terminates the process via API.
pub fn send_signal(pid: u32) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: kill with SIGTERM is a standard termination signal.
        // The process ID is validated by the OS kernel.
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret != 0 {
            anyhow::bail!(
                "Failed to send SIGTERM to process {pid}: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: use taskkill to terminate the process
        let status = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()?;
        if !status.success() {
            anyhow::bail!("Failed to terminate process {pid}");
        }
    }
    Ok(())
}

/// Blocks until a platform-appropriate shutdown signal is received.
///
/// On Unix, listens for both SIGINT (Ctrl+C) and SIGTERM.
/// On non-Unix platforms, listens for Ctrl+C only.
pub async fn wait_for_shutdown_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt())?;
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = sigint.recv() => {
                info!("Received Ctrl+C, initiating shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        info!("Received Ctrl+C, initiating shutdown...");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_pid_file_path() {
        let dir = Path::new("/tmp/test");
        let path = pid_file_path(dir);
        assert_eq!(path, PathBuf::from("/tmp/test/daemon.pid"));
    }

    #[test]
    fn test_write_and_read_pid_file() {
        let tmp = TempDir::new().unwrap();
        let path = pid_file_path(tmp.path());

        write_pid_file(&path, 12345).unwrap();
        let pid = read_pid_file(&path);
        assert_eq!(pid, Some(12345));
    }

    #[test]
    fn test_read_pid_file_missing() {
        let path = Path::new("/nonexistent/daemon.pid");
        assert_eq!(read_pid_file(path), None);
    }
}
