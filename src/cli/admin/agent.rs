//! Agent handler functions for CLI admin.

use super::common::{config_dir, json_error, json_output, AgentCreateOutput};
use crate::admin::client::admin_socket_path;
use crate::admin::{AdminClient, AdminRequest, AdminResponse};
use crate::cli::args::AgentAction;
use anyhow::Result;
use std::path::PathBuf;

pub async fn handle_agent(action: AgentAction, json: bool) -> Result<()> {
    handle_agent_with(action, config_dir(), json).await
}

pub async fn handle_agent_with(action: AgentAction, cfg_dir: PathBuf, json: bool) -> Result<()> {
    let client = AdminClient::new(admin_socket_path(&cfg_dir).to_string_lossy().into_owned());
    match action {
        AgentAction::List => handle_agent_list_rpc(&client, json).await,
        AgentAction::Info { name } => handle_agent_info_rpc(&client, &name, json).await,
        AgentAction::Create { name, model } => {
            handle_agent_create_rpc(&client, &name, model, json).await
        }
    }
}

async fn handle_agent_list_rpc(client: &AdminClient, json: bool) -> Result<()> {
    let resp = client
        .call(&AdminRequest::AgentList)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        AdminResponse::AgentListResult { agents } => {
            if agents.is_empty() {
                println!("Agents:\n  (none)");
            } else {
                println!("Agents:");
                for a in &agents {
                    let model = a.model.as_deref().unwrap_or("-");
                    println!("  {} | {} | {}", a.id, a.name, model);
                }
            }
            Ok(())
        }
        AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}

async fn handle_agent_info_rpc(client: &AdminClient, name: &str, json: bool) -> Result<()> {
    let resp = client
        .call(&AdminRequest::AgentInfo {
            name: name.to_string(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        AdminResponse::AgentInfoResult {
            id,
            name,
            model,
            skills,
        } => {
            println!("Agent: {}", name);
            println!("  ID: {}", id);
            println!("  Model: {}", model.as_deref().unwrap_or("-"));
            if skills.is_empty() {
                println!("  Skills: (none)");
            } else {
                println!("  Skills: {}", skills.join(", "));
            }
            Ok(())
        }
        AdminResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}

async fn handle_agent_create_rpc(
    client: &AdminClient,
    name: &str,
    model: Option<String>,
    json: bool,
) -> Result<()> {
    let resp = client
        .call(&AdminRequest::AgentCreate {
            name: name.to_string(),
            model,
        })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    match resp {
        AdminResponse::Ok => {
            if json {
                json_output(&AgentCreateOutput {
                    status: "created",
                    name: name.to_string(),
                });
                return Ok(());
            }
            println!("Agent '{}' created.", name);
            Ok(())
        }
        AdminResponse::Error { message } => {
            if json {
                return Err(json_error(&message));
            }
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }
}
