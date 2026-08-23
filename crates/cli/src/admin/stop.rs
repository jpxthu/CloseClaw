//! Stop handler function for CLI admin.

use super::common::{json_output, StopOutput};
use anyhow::Result;

pub async fn handle_stop(force: bool, json: bool) -> Result<()> {
    let config_dir = closeclaw_platform::config::root_dir()?;
    let p = closeclaw_platform::process::pid_file_path(&config_dir);
    let pid = match closeclaw_platform::process::read_pid_file(&p) {
        Some(pid) => pid,
        None => {
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
            return Ok(());
        }
    };
    if pid == std::process::id() {
        anyhow::bail!("Refusing to kill self.");
    }
    closeclaw_platform::process::send_signal(pid, force)?;
    let _ = std::fs::remove_file(&p);
    let sig = if force { "KILL" } else { "TERM" };
    if json {
        json_output(&StopOutput {
            pid: Some(pid),
            signal: sig.to_string(),
            stopped: true,
        });
        return Ok(());
    }
    println!("Daemon (PID {}) stopped ({}).", pid, sig);
    Ok(())
}
