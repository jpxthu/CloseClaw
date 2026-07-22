//! Session tools registrar — provides tool construction logic for [`SessionManager`].
//!
//! In the production path, session tool registration flows through
//! [`SessionManager::register_tools`] (called during daemon initialization in
//! `wire_session_manager`). The daemon injects a callback that constructs
//! these same tools and registers them into the `ToolRegistry`. This module
//! encapsulates the construction logic used by that callback.
//!
//! This struct also implements [`ToolRegistrar`] for use in tests where
//! session tools need to be registered directly into a `ToolRegistry`.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_agent::AgentConfigLookup;
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;
use closeclaw_permission::engine::engine_eval::PermissionEngine;

use crate::builtin::{SessionsKillTool, SessionsSpawnTool, SessionsSteerTool, SessionsYieldTool};
use crate::try_register;
use crate::{SpawnValidator, Tool};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Provides tool construction logic for the sessions domain.
///
/// In production, the daemon's `wire_session_manager` injects a callback
/// that constructs these tools and registers them via
/// [`SessionManager::register_tools`]. This struct encapsulates that
/// construction logic and also implements [`ToolRegistrar`] for test use.
///
/// Covers `sessions` group (4 tools):
/// `sessions_spawn`, `sessions_steer`, `sessions_kill`, `sessions_yield`.
pub struct SessionToolsRegistrar {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

impl SessionToolsRegistrar {
    /// Create a new `SessionToolsRegistrar` with the required dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        permission_engine: Arc<tokio::sync::RwLock<PermissionEngine>>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            spawn_validator,
            session_manager,
            agent_config_lookup,
            permission_engine,
            approval_flow,
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
        // Cross-reference: the tool construction order here must stay in sync
        // with `build_session_tool_callback` in
        // `crates/daemon/src/registries.rs` (which sets the callback on
        // SessionManager). If either side changes the tool set or order,
        // update the other.
        let mut registered = 0usize;
        let r = self.name();
        try_register!(
            registry,
            registered,
            SessionsSpawnTool::new(
                self.spawn_validator.clone(),
                self.session_manager.clone(),
                self.agent_config_lookup.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsSteerTool::new(
                self.session_manager.clone(),
                self.permission_engine.clone(),
                self.approval_flow.clone()
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsKillTool::new(
                self.session_manager.clone(),
                self.permission_engine.clone(),
                self.approval_flow.clone()
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsYieldTool::new(self.session_manager.clone()),
            r
        );
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 4 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
