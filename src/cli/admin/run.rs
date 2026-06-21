//! Run handler function for CLI admin.

use super::common::{json_output, RunOutput};
use anyhow::Result;
use std::path::PathBuf;

/// Start the daemon process.
///
/// Resolves the config directory, writes the current PID to disk, and
/// launches the daemon event loop.  In JSON mode a [`RunOutput`] is
/// printed instead of human-readable text.
pub async fn handle_run(config_dir: String, json: bool) -> Result<()> {
    let config_dir: PathBuf = if config_dir.is_empty() {
        crate::platform::config::config_dir()?
    } else {
        PathBuf::from(config_dir)
    };
    std::fs::create_dir_all(&config_dir)?;

    let pid = std::process::id();
    let p = crate::platform::process::pid_file_path(&config_dir);
    crate::platform::process::write_pid_file(&p, pid)?;
    println!("PID {} written to {}", pid, p.display());

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

    println!("Daemon stopped.");
    Ok(())
}
