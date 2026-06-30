//! Stop handler function for CLI admin.

use super::common::{json_output, StopOutput};
use anyhow::Result;

pub async fn handle_stop(force: bool, json: bool) -> Result<()> {
    let config_dir = closeclaw_platform::config::root_dir()?;
    let p = closeclaw_platform::process::pid_file_path(&config_dir);
    let pid = closeclaw_platform::process::read_pid_file(&p)
        .ok_or_else(|| anyhow::anyhow!("PID file not found at {}.", p.display()))?;
    if pid == std::process::id() {
        anyhow::bail!("Refusing to kill self.");
    }
    closeclaw_platform::process::send_signal(pid, force)?;
    let _ = std::fs::remove_file(&p);
    let sig = if force { "KILL" } else { "TERM" };
    if json {
        json_output(&StopOutput {
            pid,
            signal: sig.to_string(),
            stopped: true,
        });
        return Ok(());
    }
    println!("Daemon (PID {}) stopped ({}).", pid, sig);
    Ok(())
}
