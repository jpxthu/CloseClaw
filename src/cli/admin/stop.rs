//! Stop handler function for CLI admin.

use super::common::{json_error, json_output, pid_file_path, StopOutput};
use anyhow::Result;

pub async fn handle_stop(force: bool, json: bool) -> Result<()> {
    let p = pid_file_path();
    let pid: u32 = if p.exists() {
        std::fs::read_to_string(&p)?
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid PID"))?
    } else {
        anyhow::bail!("PID file not found at {}.", p.display())
    };
    if pid == std::process::id() {
        anyhow::bail!("Refusing to kill self.");
    }
    let sig = if force { "KILL" } else { "TERM" };
    match std::process::Command::new("kill")
        .arg(format!("-{}", sig))
        .arg(pid.to_string())
        .output()
    {
        Ok(o) if o.status.success() => {
            let _ = std::fs::remove_file(&p);
            if json {
                json_output(&StopOutput {
                    pid,
                    signal: sig.to_string(),
                    stopped: true,
                });
                return Ok(());
            }
            println!("Daemon (PID {}) stopped ({}).", pid, sig);
        }
        Ok(o) => {
            if json {
                return Err(json_error(&format!("kill returned {}", o.status)));
            }
            anyhow::bail!("kill returned {}", o.status);
        }
        Err(e) => {
            if json {
                return Err(json_error(&format!("Failed to kill: {}", e)));
            }
            anyhow::bail!("Failed to kill: {}", e);
        }
    }
    Ok(())
}
