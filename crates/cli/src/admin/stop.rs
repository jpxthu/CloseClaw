//! Stop handler function for CLI admin.

use std::path::Path;
use std::time::Duration;

use super::common::{json_output, StopOutput};
use anyhow::Result;

/// Production default timeout for waiting the daemon to exit after SIGTERM.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn handle_stop(json: bool) -> Result<()> {
    let pid_file = closeclaw_platform::process::pid_file_path()?;
    handle_stop_at(&pid_file, json).await
}

pub async fn handle_stop_at(pid_file: &Path, json: bool) -> Result<()> {
    handle_stop_at_with_timeout(pid_file, json, STOP_TIMEOUT).await
}

pub async fn handle_stop_at_with_timeout(
    pid_file: &Path,
    json: bool,
    timeout: Duration,
) -> Result<()> {
    // Self-kill guard: read PID before calling stop_daemon so we can bail
    // early without side effects.
    if let Some(pid) = closeclaw_platform::process::read_pid_file(pid_file) {
        if pid == std::process::id() {
            anyhow::bail!("Refusing to kill self.");
        }
    }
    let outcome = closeclaw_platform::process::stop_daemon(pid_file, timeout)?;
    match outcome {
        closeclaw_platform::process::StopOutcome::Stopped(pid) => {
            if json {
                json_output(&StopOutput {
                    pid: Some(pid),
                    signal: "TERM".to_string(),
                    stopped: true,
                });
            } else {
                println!("Daemon (PID {}) stopped (TERM).", pid);
            }
        }
        closeclaw_platform::process::StopOutcome::NotRunning => {
            let msg = format!(
                "Daemon is not running (no PID file at {}).",
                pid_file.display()
            );
            if json {
                json_output(&StopOutput {
                    pid: None,
                    signal: String::new(),
                    stopped: false,
                });
            } else {
                println!("{}", msg);
            }
        }
    }
    Ok(())
}
