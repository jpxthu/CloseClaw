//! Agent handler functions for CLI admin.

use super::common::{config_dir, json_error, json_output, AgentCreateOutput};
use crate::admin::{admin_socket_path, AdminClient, AdminRequest, AdminResponse};
use crate::args::AgentAction;
use anyhow::Result;
use std::path::PathBuf;

pub async fn handle_agent(action: AgentAction, json: bool) -> Result<()> {
    handle_agent_with(action, config_dir(), json).await
}

pub async fn handle_agent_with(action: AgentAction, cfg_dir: PathBuf, json: bool) -> Result<()> {
    let client = AdminClient::new(admin_socket_path(&cfg_dir).to_string_lossy().into_owned());
    match action {
        AgentAction::List => handle_agent_list_rpc(&client, json).await,
        AgentAction::Info { id } => handle_agent_info_rpc(&client, &id, json).await,
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

async fn handle_agent_info_rpc(client: &AdminClient, id: &str, json: bool) -> Result<()> {
    let resp = client
        .call(&AdminRequest::AgentInfo { id: id.to_string() })
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to daemon: {}", e))?;
    if json {
        json_output(&resp);
        return Ok(());
    }
    match resp {
        AdminResponse::AgentInfoResult(info) => {
            // Identity
            println!("Agent: {}", info.id);
            println!("  Name: {}", info.name);
            if let Some(ref pid) = info.parent_id {
                println!("  Parent ID: {}", pid);
            }
            // Runtime
            println!(
                "  Model: {}",
                info.model
                    .as_ref()
                    .map_or("-".to_string(), |m| m.to_string())
            );
            if let Some(ref ws) = info.workspace {
                println!("  Workspace: {}", ws);
            }
            if let Some(ref ad) = info.agent_dir {
                println!("  Agent Dir: {}", ad);
            }
            println!("  Bootstrap Mode: {}", info.bootstrap_mode);
            // Capabilities
            if info.skills.is_empty() {
                println!("  Skills: (none)");
            } else {
                println!("  Skills: {}", info.skills.join(", "));
            }
            if info.tools.is_empty() {
                println!("  Tools: (none)");
            } else {
                println!("  Tools: {}", info.tools.join(", "));
            }
            if info.disallowed_tools.is_empty() {
                println!("  Disallowed Tools: (none)");
            } else {
                println!("  Disallowed Tools: {}", info.disallowed_tools.join(", "));
            }
            // Control
            let sub = &info.subagents;
            println!("  Subagents:");
            println!("    Allow Agents: {}", sub.allow_agents.join(", "));
            if let Some(v) = sub.require_agent_id {
                println!("    Require Agent ID: {}", v);
            }
            if let Some(v) = sub.max_spawn_depth {
                println!("    Max Spawn Depth: {}", v);
            }
            if let Some(v) = sub.max_children {
                println!("    Max Children: {}", v);
            }
            if let Some(v) = sub.timeout {
                println!("    Timeout: {}s", v);
            }
            if let Some(v) = sub.timeout_warning {
                println!("    Timeout Warning: {}s", v);
            }
            if let Some(v) = sub.timeout_notify_interval_ratio {
                println!("    Timeout Notify Ratio: {}", v);
            }
            if let Some(ref m) = sub.model {
                println!("    Model: {}", m);
            }
            match info.memory {
                Some(ref mem) => {
                    println!("  Memory:");
                    println!("    Storage: {:#?}", mem.storage);
                    println!("    Mining: {:#?}", mem.mining);
                    println!("    Dreaming: {:#?}", mem.dreaming);
                    println!("    Search: {:#?}", mem.search);
                    println!("    Forgetting: {:#?}", mem.forgetting);
                }
                None => {
                    println!("  Memory: null");
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
