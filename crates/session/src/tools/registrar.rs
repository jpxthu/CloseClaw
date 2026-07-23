//! Session tools registrar — registers session management tools into the
//! [`ToolRegistry`].
//!
//! This struct implements [`ToolRegistrar`] so that session tools are
//! registered during daemon startup via the unified `register_all` flow,
//! as described in `docs/design/tools/tool-registrar.md`.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::permission_types::{SharedApprovalSubmission, SharedPermissionEvaluator};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError, ToolRegistry};
use closeclaw_common::tool_trait::Tool;
use closeclaw_config::spawn_validation::SpawnValidator;

use super::{
    LateBoundSessionManagerOps, SessionsKillTool, SessionsSpawnTool, SessionsSteerTool,
    SessionsYieldTool,
};

/// Register a tool and increment the counter on success.
///
/// Calls [`register_single`] and increments `$registered` when the tool
/// is accepted. Uses `closeclaw_common::tool_registry::register_single`
/// directly so it works from any crate.
macro_rules! try_register {
    ($registry:expr, $registered:expr, $tool:expr, $registrar_name:expr) => {
        let tool = $tool;
        let name = tool.name().to_string();
        if closeclaw_common::tool_registry::register_single($registry, name, tool, $registrar_name)
            .await?
        {
            $registered += 1;
        }
    };
}

/// Registers session management tools (`sessions` group) into the
/// [`ToolRegistry`].
///
/// Covers 4 tools:
/// `sessions_spawn`, `sessions_steer`, `sessions_kill`, `sessions_yield`.
pub struct SessionToolsRegistrar {
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<LateBoundSessionManagerOps>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    permission_engine: SharedPermissionEvaluator,
    approval_flow: SharedApprovalSubmission,
}

impl SessionToolsRegistrar {
    /// Create a new `SessionToolsRegistrar` with the required dependencies.
    pub fn new(
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<LateBoundSessionManagerOps>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        permission_engine: SharedPermissionEvaluator,
        approval_flow: SharedApprovalSubmission,
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

    async fn register(&self, registry: &dyn ToolRegistry) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        try_register!(
            registry,
            registered,
            SessionsSpawnTool::new(
                self.spawn_validator.clone(),
                Arc::clone(&self.session_manager) as Arc<dyn super::SessionManagerOps>,
                self.agent_config_lookup.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsSteerTool::new(
                Arc::clone(&self.session_manager) as Arc<dyn super::SessionManagerOps>,
                self.permission_engine.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsKillTool::new(
                Arc::clone(&self.session_manager) as Arc<dyn super::SessionManagerOps>,
                self.permission_engine.clone(),
                self.approval_flow.clone(),
            ),
            r
        );
        try_register!(
            registry,
            registered,
            SessionsYieldTool::new(
                Arc::clone(&self.session_manager) as Arc<dyn super::SessionManagerOps>
            ),
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
