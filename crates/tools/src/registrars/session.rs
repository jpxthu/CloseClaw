//! Session tools registrar — registers sessions_spawn, sessions_steer, sessions_kill.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_agent::AgentConfigLookup;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::engine::engine_eval::PermissionEngine;

use crate::builtin::{SessionsKillTool, SessionsSpawnTool, SessionsSteerTool};
use crate::try_register;
use crate::{SpawnValidator, Tool};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Session tools registrar — registers all tools from the sessions domain.
///
/// Covers `sessions` group (3 tools):
/// `sessions_spawn`, `sessions_steer`, `sessions_kill`.
pub struct SessionToolsRegistrar {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    permission_engine: Arc<PermissionEngine>,
}

impl SessionToolsRegistrar {
    /// Create a new `SessionToolsRegistrar` with the required dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        permission_engine: Arc<PermissionEngine>,
    ) -> Self {
        Self {
            spawn_validator,
            session_manager,
            agent_config_lookup,
            permission_engine,
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

    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        try_register!(
            registry,
            registered,
            SessionsSpawnTool::new(
                self.spawn_validator.clone(),
                self.session_manager.clone(),
                self.agent_config_lookup.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsSteerTool::new(self.session_manager.clone(), self.permission_engine.clone()),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsKillTool::new(self.session_manager.clone(), self.permission_engine.clone()),
            r
        );
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 3 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
