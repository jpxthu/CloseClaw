//! Session tools registrar — registers sessions_spawn, sessions_steer, sessions_kill.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::AgentConfigLookup;
use closeclaw_gateway::SessionManager;

use crate::builtin::{SessionsKillTool, SessionsSpawnTool, SessionsSteerTool};
use crate::registrar::{ToolRegistrar, ToolRegistrarError};
use crate::{SpawnValidator, ToolRegistry};

/// Session tools registrar — registers all tools from the sessions domain.
///
/// Covers `sessions` group (3 tools):
/// `sessions_spawn`, `sessions_steer`, `sessions_kill`.
pub struct SessionToolsRegistrar {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
}

impl SessionToolsRegistrar {
    /// Create a new `SessionToolsRegistrar` with the required dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
    ) -> Self {
        Self {
            spawn_validator,
            session_manager,
            agent_config_lookup,
        }
    }
}

#[async_trait]
impl ToolRegistrar for SessionToolsRegistrar {
    fn name(&self) -> &str {
        "SessionToolsRegistrar"
    }

    fn priority(&self) -> u32 {
        2
    }

    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        register_tool(
            registry,
            SessionsSpawnTool::new(
                self.spawn_validator.clone(),
                self.session_manager.clone(),
                self.agent_config_lookup.clone(),
            ),
        )
        .await?;
        register_tool(
            registry,
            SessionsSteerTool::new(self.session_manager.clone()),
        )
        .await?;
        register_tool(
            registry,
            SessionsKillTool::new(self.session_manager.clone()),
        )
        .await?;

        Ok(())
    }
}

/// Helper: register a single tool, converting `ToolError` into
/// `ToolRegistrarError`.
async fn register_tool(
    registry: &ToolRegistry,
    tool: impl crate::Tool + 'static,
) -> Result<(), ToolRegistrarError> {
    registry.register(tool).await.map_err(|e| match e {
        crate::ToolError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
            tool: name,
            registrar: "SessionToolsRegistrar".to_string(),
        },
        other => ToolRegistrarError::Internal(other.to_string()),
    })
}
