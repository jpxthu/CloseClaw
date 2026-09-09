//! Skill handler functions for CLI admin.

use super::common::{config_dir, json_error, json_output, SkillRescanOutput};
use crate::admin::{admin_socket_path, AdminClient, AdminRequest, AdminResponse};
use crate::args::SkillAction;
use anyhow::Result;
use std::path::PathBuf;

pub async fn handle_skill(action: SkillAction, json: bool) -> Result<()> {
    handle_skill_with(action, config_dir(), json).await
}

pub async fn handle_skill_with(action: SkillAction, cfg_dir: PathBuf, json: bool) -> Result<()> {
    let client = AdminClient::new(admin_socket_path(&cfg_dir).to_string_lossy().into_owned());
    match action {
        SkillAction::List => handle_skill_list_rpc(&client, json).await,
        SkillAction::Rescan => handle_skill_rescan_rpc(&client, json).await,
    }
}

async fn handle_skill_list_rpc(client: &AdminClient, json: bool) -> Result<()> {
    let resp = client
        .call(&AdminRequest::SkillList)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        AdminResponse::SkillListResult { skills } => {
            if skills.is_empty() {
                println!("Installed skills:\n  (none)");
            } else {
                println!("Installed skills:");
                for s in &skills {
                    let ver = s.version.as_deref().unwrap_or("-");
                    println!("  {} v{}", s.name, ver);
                }
            }
        }
        AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
    Ok(())
}

async fn handle_skill_rescan_rpc(client: &AdminClient, json: bool) -> Result<()> {
    let resp = client
        .call(&AdminRequest::SkillRescan)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    match resp {
        AdminResponse::SkillRescanResult { count } => {
            if json {
                json_output(&SkillRescanOutput {
                    status: "rescanned",
                    count,
                });
                return Ok(());
            }
            println!("Rescanned skills: {} loaded.", count);
        }
        AdminResponse::Error { message } => {
            if json {
                return Err(json_error(&message));
            }
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
    Ok(())
}
