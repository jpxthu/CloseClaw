//! Plan tools registrar — registers ProgressTool, PlanApprovalTool, and ExecutePlanTool.
//!
//! Registers the ProgressTool for plan execution step tracking,
//! PlanApprovalTool for plan approval gates, and ExecutePlanTool
//! for natural-language plan execution triggering.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use closeclaw_agent::AgentConfigLookup;
use closeclaw_common::{PlanState, PlanStateWriter};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;

use crate::builtin::execute_plan::ExecutePlanTool;
use crate::builtin::plan_approval::PlanApprovalTool;
use crate::builtin::ProgressTool;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Plan tools registrar — registers tools from the plan domain.
///
/// Covers the `plan` group (3 tools): `ProgressTool`, `PlanApprovalTool`,
/// and `ExecutePlanTool`.
pub struct PlanToolsRegistrar {
    plan_state: Arc<Mutex<PlanState>>,
    writer: Option<Arc<dyn PlanStateWriter>>,
    session_manager: Arc<SessionManager>,
    agent_config_lookup: Arc<dyn AgentConfigLookup>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

impl PlanToolsRegistrar {
    /// Create a new `PlanToolsRegistrar` with the given shared `PlanState`.
    pub fn new(
        plan_state: Arc<Mutex<PlanState>>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            plan_state,
            writer: None,
            session_manager,
            agent_config_lookup,
            approval_flow,
        }
    }

    /// Create a new `PlanToolsRegistrar` with a [`PlanStateWriter`] for
    /// plan file synchronization.
    pub fn with_writer(
        plan_state: Arc<Mutex<PlanState>>,
        writer: Arc<dyn PlanStateWriter>,
        session_manager: Arc<SessionManager>,
        agent_config_lookup: Arc<dyn AgentConfigLookup>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            plan_state,
            writer: Some(writer),
            session_manager,
            agent_config_lookup,
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
        let progress_tool = match &self.writer {
            Some(w) => ProgressTool::with_writer(Arc::clone(&self.plan_state), Arc::clone(w)),
            None => ProgressTool::new(Arc::clone(&self.plan_state)),
        };
        try_register!(registry, registered, progress_tool, r);
        let plan_approval = PlanApprovalTool::new();
        try_register!(registry, registered, plan_approval, r);
        let execute_plan = ExecutePlanTool::new(
            Arc::clone(&self.session_manager),
            Arc::clone(&self.agent_config_lookup),
            Arc::clone(&self.approval_flow),
        );
        try_register!(registry, registered, execute_plan, r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 3 plan tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
