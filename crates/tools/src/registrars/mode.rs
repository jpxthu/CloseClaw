//! Mode tools registrar — registers ModeExecutionTriggerTool.
//!
//! Registers ModeExecutionTriggerTool for natural-language plan execution triggering.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_gateway::SessionManager;

use crate::builtin::mode_execution_trigger::ModeExecutionTriggerTool;
use crate::builtin::PlanExecConfirmFlow;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Mode tools registrar — registers tools from the mode domain.
///
/// Covers the `mode` group (1 tool): `ModeExecutionTriggerTool`.
pub struct ModeToolsRegistrar {
    session_manager: Arc<SessionManager>,
    confirm_flow: Arc<PlanExecConfirmFlow>,
}

impl ModeToolsRegistrar {
    /// Create a new `ModeToolsRegistrar`.
    pub fn new(
        session_manager: Arc<SessionManager>,
        confirm_flow: Arc<PlanExecConfirmFlow>,
    ) -> Self {
        Self {
            session_manager,
            confirm_flow,
        }
    }
}

#[async_trait]
impl ToolRegistrar for ModeToolsRegistrar {
    fn name(&self) -> &str {
        "ModeToolsRegistrar"
    }

    fn priority(&self) -> u32 {
        3
    }

    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        let mode_exec_trigger = ModeExecutionTriggerTool::new(
            Arc::clone(&self.session_manager),
            Arc::clone(&self.confirm_flow),
        );
        try_register!(registry, registered, mode_exec_trigger, r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 1 mode tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
