//! Admin RPC server — listens on a Unix domain socket and dispatches
//! requests to daemon components.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};

use crate::admin::protocol::{AdminRequest, AdminResponse, SkillInfo};
use crate::agent::config::AgentConfig;
use crate::agent::registry::AgentRegistry;
use crate::config::manager::write_atomically;
use crate::config::ConfigManager;
use crate::skills::DiskSkillRegistry;

/// Server-side context holding references to daemon components.
pub struct AdminContext {
    pub agent_registry: Arc<AgentRegistry>,
    pub skill_registry: Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>,
    pub config_manager: Arc<ConfigManager>,
    pub config_dir: PathBuf,
}

/// Admin RPC server that binds a Unix domain socket and handles
/// incoming requests.
pub struct AdminServer {
    path: PathBuf,
    context: Arc<AdminContext>,
}

impl AdminServer {
    /// Create a new admin server with the given socket path and context.
    pub fn new(path: impl Into<PathBuf>, context: AdminContext) -> Self {
        Self {
            path: path.into(),
            context: Arc::new(context),
        }
    }

    /// Remove the socket file if it already exists (idempotent).
    async fn clean_up(&self) {
        let _ = tokio::fs::remove_file(&self.path).await;
    }

    /// Start the admin server. Blocks forever, processing each
    /// connection in a spawned task.
    pub async fn serve(self) -> std::io::Result<()> {
        self.clean_up().await;

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let listener = UnixListener::bind(&self.path)?;

        tracing::info!("admin RPC server listening on {}", self.path.display());

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let context = Arc::clone(&self.context);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, context).await {
                            tracing::error!("admin RPC connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("admin RPC accept error: {}", e);
                }
            }
        }
    }
}

/// Handle a single admin RPC connection.
async fn handle_connection(stream: UnixStream, context: Arc<AdminContext>) -> std::io::Result<()> {
    let (reader, mut writer): (_, OwnedWriteHalf) = stream.into_split();
    let mut reader = BufReader::new(reader);

    loop {
        // Read 4-byte length header
        let mut hdr = [0u8; 4];
        match reader.read_exact(&mut hdr).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let body_len = u32::from_be_bytes(hdr) as usize;

        // Read body
        let mut body = vec![0u8; body_len];
        reader.read_exact(&mut body).await?;

        // Deserialize request
        let request: AdminRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = AdminResponse::Error {
                    message: format!("invalid request: {}", e),
                };
                send_response(&mut writer, &resp).await?;
                continue;
            }
        };

        // Dispatch request
        let response = dispatch(request, &context).await;
        send_response(&mut writer, &response).await?;
    }

    Ok(())
}

/// Dispatch a request to the appropriate handler.
async fn dispatch(request: AdminRequest, context: &AdminContext) -> AdminResponse {
    match request {
        AdminRequest::AgentList => dispatch_agent_list(context),
        AdminRequest::AgentInfo { name } => dispatch_agent_info(&name, context),
        AdminRequest::AgentCreate { name, model } => {
            dispatch_agent_create(&name, model, context).await
        }
        AdminRequest::SkillList => dispatch_skill_list(context).await,
        AdminRequest::SkillInstall { name } => dispatch_skill_install(&name, context).await,
        AdminRequest::Ping => AdminResponse::Pong,
    }
}

/// List all agents — returns agent ID + model for each registered agent.
fn dispatch_agent_list(context: &AdminContext) -> AdminResponse {
    let agents: Vec<crate::admin::protocol::AgentInfo> = context
        .agent_registry
        .iter()
        .map(|entry| crate::admin::protocol::AgentInfo {
            id: entry.key().clone(),
            name: entry.name.clone(),
            model: entry.model.clone(),
        })
        .collect();
    AdminResponse::AgentListResult { agents }
}

/// Get info for a specific agent — returns detailed config information.
fn dispatch_agent_info(name: &str, context: &AdminContext) -> AdminResponse {
    match context.agent_registry.get(name) {
        Some(entry) => AdminResponse::AgentInfoResult {
            id: entry.id.clone(),
            name: entry.name.clone(),
            model: entry.model.clone(),
            skills: entry.skills.clone(),
        },
        None => AdminResponse::Error {
            message: format!("agent '{}' not found", name),
        },
    }
}

/// Validate that the agent name is non-empty and not already taken.
fn validate_agent_name(name: &str, context: &AdminContext) -> Result<(), AdminResponse> {
    if name.is_empty() {
        return Err(AdminResponse::Error {
            message: "agent name cannot be empty".to_string(),
        });
    }
    if context.agent_registry.get(name).is_some() {
        return Err(AdminResponse::Error {
            message: format!("agent '{}' already exists", name),
        });
    }
    Ok(())
}

/// Create the agent directory and write config.json, returning the agent dir path.
async fn create_agent_dir_and_config(
    name: &str,
    model: Option<String>,
    context: &AdminContext,
) -> Result<std::path::PathBuf, AdminResponse> {
    let agent_dir = context.config_dir.join("agents").join(name);
    tokio::fs::create_dir_all(&agent_dir)
        .await
        .map_err(|e| AdminResponse::Error {
            message: format!("failed to create agent directory: {}", e),
        })?;
    let config = AgentConfig {
        id: name.to_string(),
        model,
        ..AgentConfig::default()
    };
    let config_path = agent_dir.join("config.json");
    config
        .save(&config_path)
        .map_err(|e| AdminResponse::Error {
            message: format!("failed to write config.json: {}", e),
        })?;
    Ok(agent_dir)
}

/// Append the new agent name to agents.json.
fn update_agents_json(name: &str, context: &AdminContext) -> Result<(), AdminResponse> {
    let agents_json_path = context.config_dir.join("config").join("agents.json");
    let mut agent_ids = context
        .config_manager
        .load_agents_json(&agents_json_path)
        .unwrap_or_default();
    agent_ids.push(name.to_string());
    let agents_json = serde_json::json!({ "agents": agent_ids });
    let agents_json_bytes =
        serde_json::to_vec_pretty(&agents_json).map_err(|e| AdminResponse::Error {
            message: format!("failed to serialize agents.json: {}", e),
        })?;
    write_atomically(&agents_json_path, &agents_json_bytes).map_err(|e| AdminResponse::Error {
        message: format!("failed to update agents.json: {}", e),
    })
}

/// Reload agent configs and repopulate the registry.
fn reload_registry(context: &AdminContext) -> Result<(), AdminResponse> {
    context
        .config_manager
        .reload_agents()
        .map_err(|e| AdminResponse::Error {
            message: format!("failed to reload agent configs: {}", e),
        })?;
    let configs: Vec<_> = context.config_manager.agents().into_values().collect();
    context.agent_registry.populate(configs);
    Ok(())
}

/// Create a new agent — creates config file and registers in AgentRegistry.
async fn dispatch_agent_create(
    name: &str,
    model: Option<String>,
    context: &AdminContext,
) -> AdminResponse {
    if let Err(e) = validate_agent_name(name, context) {
        return e;
    }
    if let Err(e) = create_agent_dir_and_config(name, model, context).await {
        return e;
    }
    if let Err(e) = update_agents_json(name, context) {
        return e;
    }
    if let Err(e) = reload_registry(context) {
        return e;
    }
    tracing::info!(name = name, "agent created successfully");
    AdminResponse::Ok
}

/// List all skills from the DiskSkillRegistry with version info.
async fn dispatch_skill_list(context: &AdminContext) -> AdminResponse {
    let guard = context
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(registry) => {
            let skills: Vec<SkillInfo> = registry
                .list()
                .into_iter()
                .map(|name| SkillInfo {
                    name: name.to_string(),
                    version: None,
                })
                .collect();
            AdminResponse::SkillListResult { skills }
        }
        None => AdminResponse::SkillListResult { skills: vec![] },
    }
}

/// Install a skill from the global skills directory to the bundled directory.
async fn dispatch_skill_install(name: &str, context: &AdminContext) -> AdminResponse {
    // Derive global dir: parent of config_dir / skills
    let global_dir = context.config_dir.parent().map(|p| p.join("skills"));
    let bundled_dir = context.config_dir.join("skills");

    let global_dir = match global_dir {
        Some(d) => d,
        None => {
            return AdminResponse::Error {
                message: "cannot determine global skills directory".to_string(),
            }
        }
    };

    // Source: global skills directory
    let source_skill_dir = global_dir.join(name);
    if !source_skill_dir.exists() {
        return AdminResponse::Error {
            message: format!(
                "skill '{}' not found in global directory {}",
                name,
                global_dir.display()
            ),
        };
    }

    // Check SKILL.md exists
    let source_skill_md = source_skill_dir.join("SKILL.md");
    if !source_skill_md.exists() {
        return AdminResponse::Error {
            message: format!("skill '{}' does not contain SKILL.md", name),
        };
    }

    // Destination: bundled skills directory
    let dest_skill_dir = bundled_dir.join(name);
    if dest_skill_dir.exists() {
        return AdminResponse::Error {
            message: format!("skill '{}' is already installed", name),
        };
    }

    // Copy skill directory
    if let Err(e) = copy_skill_dir(&source_skill_dir, &dest_skill_dir).await {
        return AdminResponse::Error {
            message: format!("failed to copy skill: {}", e),
        };
    }

    tracing::info!(name = name, "skill installed successfully");
    AdminResponse::Ok
}

/// Recursively copy a skill directory using async I/O.
async fn copy_skill_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dst).await?;
    let mut entries = tokio::fs::read_dir(src).await?;
    while let Some(entry) = entries.next_entry().await? {
        let file_type = entry.file_type().await?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            Box::pin(copy_skill_dir(&src_path, &dst_path)).await?;
        } else {
            tokio::fs::copy(&src_path, &dst_path).await?;
        }
    }
    Ok(())
}

/// Send a length-prefixed JSON response.
async fn send_response(
    writer: &mut OwnedWriteHalf,
    response: &AdminResponse,
) -> std::io::Result<()> {
    let json = serde_json::to_vec(response)?;
    let len = (json.len() as u32).to_be_bytes();
    writer.write_all(&len).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::registry::AgentRegistry;
    use crate::skills::DiskSkillRegistry;

    fn make_test_context() -> AdminContext {
        let config_dir = tempfile::tempdir().unwrap().keep();
        // Create config/agents.json so ConfigManager can load
        let config_sub = config_dir.join("config");
        std::fs::create_dir_all(&config_sub).unwrap();
        std::fs::write(config_sub.join("agents.json"), r#"{"agents": []}"#).unwrap();
        let config_manager =
            Arc::new(crate::config::ConfigManager::new(config_dir.clone()).unwrap());
        AdminContext {
            agent_registry: Arc::new(AgentRegistry::new()),
            skill_registry: Arc::new(std::sync::RwLock::new(Some(DiskSkillRegistry::default()))),
            config_manager,
            config_dir,
        }
    }

    #[test]
    fn test_dispatch_agent_list_empty() {
        let ctx = make_test_context();
        let resp = dispatch_agent_list(&ctx);
        match resp {
            AdminResponse::AgentListResult { agents } => assert!(agents.is_empty()),
            _ => panic!("expected AgentListResult"),
        }
    }

    #[test]
    fn test_dispatch_agent_info_not_found() {
        let ctx = make_test_context();
        let resp = dispatch_agent_info("nonexistent", &ctx);
        match resp {
            AdminResponse::Error { message } => {
                assert!(message.contains("not found"));
            }
            _ => panic!("expected Error"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_skill_list_empty() {
        let ctx = make_test_context();
        let resp = dispatch_skill_list(&ctx).await;
        match resp {
            AdminResponse::SkillListResult { skills } => assert!(skills.is_empty()),
            _ => panic!("expected SkillListResult"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_skill_install_not_found() {
        let ctx = make_test_context();
        let resp = dispatch_skill_install("test-skill", &ctx).await;
        match resp {
            AdminResponse::Error { message } => {
                assert!(message.contains("not found"));
            }
            _ => panic!("expected Error for missing skill"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_agent_create_empty_name() {
        let ctx = make_test_context();
        let resp = dispatch_agent_create("", Some("gpt-4".into()), &ctx).await;
        match resp {
            AdminResponse::Error { message } => {
                assert!(message.contains("empty"));
            }
            _ => panic!("expected Error for empty name"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_agent_create_success() {
        let ctx = make_test_context();
        let resp = dispatch_agent_create("test-agent", Some("gpt-4".into()), &ctx).await;
        assert!(matches!(resp, AdminResponse::Ok));
        // Verify agent was created
        assert!(ctx.agent_registry.get("test-agent").is_some());
    }

    #[tokio::test]
    async fn test_dispatch_agent_create_duplicate() {
        let ctx = make_test_context();
        let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
        assert!(matches!(resp, AdminResponse::Ok));
        // Try creating same agent again
        let resp = dispatch_agent_create("dup-agent", None, &ctx).await;
        match resp {
            AdminResponse::Error { message } => {
                assert!(message.contains("already exists"));
            }
            _ => panic!("expected Error for duplicate"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_ping() {
        let ctx = make_test_context();
        let resp = dispatch(AdminRequest::Ping, &ctx).await;
        assert!(matches!(resp, AdminResponse::Pong));
    }
}
