//! Run handler function for CLI admin.

use super::common::{json_output, RunOutput};
use anyhow::Result;
use std::path::PathBuf;

/// Returns the resolved config directory, the current process PID, and the
/// path to the PID file.
///
/// Separated from [`handle_run`] so that tests can verify directory
/// resolution and PID writing without starting a real daemon.
pub fn prepare_run(config_dir: &str) -> Result<(PathBuf, u32, PathBuf)> {
    let config_dir: PathBuf = if config_dir.is_empty() {
        crate::platform::config::config_dir()?
    } else {
        PathBuf::from(config_dir)
    };
    std::fs::create_dir_all(&config_dir)?;

    let pid = std::process::id();
    let pid_file = crate::platform::process::pid_file_path(&config_dir);
    crate::platform::process::write_pid_file(&pid_file, pid)?;

    Ok((config_dir, pid, pid_file))
}

/// Start the daemon process.
///
/// Resolves the config directory, writes the current PID to disk, and
/// launches the daemon event loop.  In JSON mode a [`RunOutput`] is
/// printed instead of human-readable text.
pub async fn handle_run(config_dir: String, json: bool) -> Result<()> {
    let (config_dir, pid, pid_file) = prepare_run(&config_dir)?;

    crate::daemon::Daemon::start(config_dir.to_string_lossy().as_ref())
        .await?
        .run()
        .await?;

    if json {
        json_output(&RunOutput {
            pid,
            config_dir: config_dir.to_string_lossy().to_string(),
            stopped: true,
        });
        return Ok(());
    }

    println!("PID {} written to {}", pid, pid_file.display());
    println!("Daemon stopped.");
    Ok(())
}
