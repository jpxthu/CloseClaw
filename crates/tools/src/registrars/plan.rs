//! Plan tools registrar — registers ExecutePlanTool.
//!
//! Registers ExecutePlanTool for natural-language plan execution triggering.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;

use crate::builtin::execute_plan::ExecutePlanTool;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Plan tools registrar — registers tools from the plan domain.
///
/// Covers the `plan` group (1 tool): `ExecutePlanTool`.
pub struct PlanToolsRegistrar {
    session_manager: Arc<SessionManager>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

impl PlanToolsRegistrar {
    /// Create a new `PlanToolsRegistrar`.
    pub fn new(
        session_manager: Arc<SessionManager>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            session_manager,
            approval_flow,
        }
    }
}

#[async_trait]
impl ToolRegistrar for PlanToolsRegistrar {
    fn name(&self) -> &str {
        "PlanToolsRegistrar"
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
        let execute_plan = ExecutePlanTool::new(
            Arc::clone(&self.session_manager),
            Arc::clone(&self.approval_flow),
        );
        try_register!(registry, registered, execute_plan, r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 1 plan tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
