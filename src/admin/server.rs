//! Admin RPC server — listens on a Unix domain socket and dispatches
//! requests to daemon components.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};

use crate::admin::protocol::{AdminRequest, AdminResponse, SkillInfo};
use crate::agent::registry::AgentRegistry;
use crate::skills::DiskSkillRegistry;

/// Server-side context holding references to daemon components.
pub struct AdminContext {
    pub agent_registry: Arc<AgentRegistry>,
    pub skill_registry: Arc<std::sync::RwLock<Option<DiskSkillRegistry>>>,
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
    fn clean_up(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Start the admin server. Blocks forever, processing each
    /// connection in a spawned task.
    pub async fn serve(self) -> std::io::Result<()> {
        self.clean_up();

        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
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
        AdminRequest::SkillInstall { name } => dispatch_skill_install(&name).await,
        AdminRequest::Ping => AdminResponse::Pong,
    }
}

/// List all agents — stub implementation (returns empty list for now).
fn dispatch_agent_list(_context: &AdminContext) -> AdminResponse {
    // TODO: iterate AgentRegistry.configs in Step 1.4
    AdminResponse::AgentListResult { agents: vec![] }
}

/// Get info for a specific agent — stub implementation.
fn dispatch_agent_info(name: &str, _context: &AdminContext) -> AdminResponse {
    // TODO: query AgentRegistry.get(name) in Step 1.4
    AdminResponse::Error {
        message: format!("agent '{}' not found (stub)", name),
    }
}

/// Create a new agent — stub implementation (returns Ok for now).
async fn dispatch_agent_create(
    name: &str,
    _model: Option<String>,
    _context: &AdminContext,
) -> AdminResponse {
    // TODO: implement via ConfigManager in Step 1.4
    tracing::info!(name = name, "agent create requested (stub)");
    AdminResponse::Ok
}

/// List all skills from the DiskSkillRegistry.
async fn dispatch_skill_list(context: &AdminContext) -> AdminResponse {
    let guard = context.skill_registry.read().unwrap();
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

/// Install a skill — stub implementation (returns Ok for now).
async fn dispatch_skill_install(name: &str) -> AdminResponse {
    // TODO: implement in Step 1.4
    tracing::info!(name = name, "skill install requested (stub)");
    AdminResponse::Ok
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
        AdminContext {
            agent_registry: Arc::new(AgentRegistry::new()),
            skill_registry: Arc::new(std::sync::RwLock::new(Some(DiskSkillRegistry::default()))),
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
    async fn test_dispatch_skill_install_stub() {
        let resp = dispatch_skill_install("test-skill").await;
        assert!(matches!(resp, AdminResponse::Ok));
    }

    #[tokio::test]
    async fn test_dispatch_agent_create_stub() {
        let ctx = make_test_context();
        let resp = dispatch_agent_create("new-agent", Some("gpt-4".into()), &ctx).await;
        assert!(matches!(resp, AdminResponse::Ok));
    }

    #[tokio::test]
    async fn test_dispatch_ping() {
        let ctx = make_test_context();
        let resp = dispatch(AdminRequest::Ping, &ctx).await;
        assert!(matches!(resp, AdminResponse::Pong));
    }
}
