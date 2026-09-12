//! Restart handler function for CLI admin.

use super::common::json_output;
use super::rpc::{admin_socket_path, AdminClient, AdminRequest, AdminResponse};
use anyhow::Result;
use serde::Serialize;

#[derive(Serialize)]
pub struct RestartOutput {
    pub status: &'static str,
    pub force: bool,
    pub message: String,
}

pub async fn handle_restart(force: bool, json: bool) -> Result<()> {
    let mgr =
        closeclaw_config::ConfigManager::with_default_root_dir().map_err(|e| anyhow::anyhow!(e))?;
    let root_dir = mgr
        .config_dir()
        .parent()
        .unwrap_or(mgr.config_dir())
        .to_path_buf();
    handle_restart_at(&root_dir, force, json).await
}

pub async fn handle_restart_at(
    config_dir: &std::path::Path,
    force: bool,
    json: bool,
) -> Result<()> {
    let sock = admin_socket_path(config_dir);
    let client = AdminClient::new(sock.to_string_lossy());
    let request = if force {
        AdminRequest::ForceRestart
    } else {
        AdminRequest::CancelPendingRestart
    };
    let resp = client
        .call(&request)
        .await
        .map_err(|e| anyhow::anyhow!("failed to connect to daemon (is it running?): {}", e))?;
    match resp {
        AdminResponse::Ok => {
            let msg = if force {
                "Gateway restart triggered (force).".to_string()
            } else {
                "Pending restart cancelled.".to_string()
            };
            if json {
                json_output(&RestartOutput {
                    status: "ok",
                    force,
                    message: msg,
                });
            } else {
                println!("{}", msg);
            }
        }
        AdminResponse::Error { message } => {
            if json {
                json_output(&RestartOutput {
                    status: "error",
                    force,
                    message,
                });
            } else {
                eprintln!("Error: {}", message);
                std::process::exit(1);
            }
        }
        other => {
            anyhow::bail!("unexpected response from daemon: {:?}", other);
        }
    }
    Ok(())
}
