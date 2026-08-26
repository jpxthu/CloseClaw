//! Stop handler function for CLI admin.

use super::common::{json_output, StopOutput};
use anyhow::Result;

pub async fn handle_stop(force: bool, json: bool) -> Result<()> {
    let config_dir = closeclaw_platform::config::root_dir()?;
    handle_stop_at(&config_dir, force, json).await
}

pub async fn handle_stop_at(config_dir: &std::path::Path, force: bool, json: bool) -> Result<()> {
    let p = closeclaw_platform::process::pid_file_path(config_dir);
    // Self-kill guard: read PID before calling stop_daemon so we can bail
    // early without side effects.
    if let Some(pid) = closeclaw_platform::process::read_pid_file(&p) {
        if pid == std::process::id() {
            anyhow::bail!("Refusing to kill self.");
        }
    }
    let outcome =
        closeclaw_platform::process::stop_daemon(&p, force, std::time::Duration::from_secs(5))?;
    let sig = if force { "KILL" } else { "TERM" };
    match outcome {
        closeclaw_platform::process::StopOutcome::Stopped(pid) => {
            if json {
                json_output(&StopOutput {
                    pid: Some(pid),
                    signal: sig.to_string(),
                    stopped: true,
                });
            } else {
                println!("Daemon (PID {}) stopped ({}).", pid, sig);
            }
        }
        closeclaw_platform::process::StopOutcome::NotRunning => {
            let msg = format!("Daemon is not running (no PID file at {}).", p.display());
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
