//! Plan tools registrar — registers ProgressTool and ExecutePlanTool.
//!
//! Registers the ProgressTool for plan execution step tracking
//! and ExecutePlanTool for natural-language plan execution triggering.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use closeclaw_common::PlanState;
use closeclaw_execution::{ExecutionState, PlanStateWriter};
use closeclaw_gateway::SessionManager;
use closeclaw_permission::approval_flow::ApprovalFlow;

use crate::builtin::execute_plan::ExecutePlanTool;
use crate::builtin::ProgressTool;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Plan tools registrar — registers tools from the plan domain.
///
/// Covers the `plan` group (2 tools): `ProgressTool` and `ExecutePlanTool`.
pub struct PlanToolsRegistrar {
    execution_state: Arc<Mutex<ExecutionState>>,
    plan_state: Arc<Mutex<PlanState>>,
    writer: Option<Arc<dyn PlanStateWriter>>,
    session_manager: Arc<SessionManager>,
    approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
}

impl PlanToolsRegistrar {
    /// Create a new `PlanToolsRegistrar` with the given shared `PlanState`.
    pub fn new(
        plan_state: Arc<Mutex<PlanState>>,
        session_manager: Arc<SessionManager>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            execution_state: Arc::new(Mutex::new(ExecutionState::new())),
            plan_state,
            writer: None,
            session_manager,
            approval_flow,
        }
    }

    /// Create a new `PlanToolsRegistrar` with a [`PlanStateWriter`] for
    /// plan file synchronization.
    pub fn with_writer(
        plan_state: Arc<Mutex<PlanState>>,
        writer: Arc<dyn PlanStateWriter>,
        session_manager: Arc<SessionManager>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            execution_state: Arc::new(Mutex::new(ExecutionState::new())),
            plan_state,
            writer: Some(writer),
            session_manager,
            approval_flow,
        }
    }

    /// Create with a pre-existing shared execution state.
    pub fn with_execution_state(
        execution_state: Arc<Mutex<ExecutionState>>,
        plan_state: Arc<Mutex<PlanState>>,
        session_manager: Arc<SessionManager>,
        approval_flow: Arc<tokio::sync::Mutex<ApprovalFlow>>,
    ) -> Self {
        Self {
            execution_state,
            plan_state,
            writer: None,
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
        let progress_tool = match &self.writer {
            Some(w) => ProgressTool::with_writer(
                Arc::clone(&self.execution_state),
                Arc::clone(&self.plan_state),
                Arc::clone(w),
            ),
            None => ProgressTool::new(
                Arc::clone(&self.execution_state),
                Arc::clone(&self.plan_state),
            ),
        };
        try_register!(registry, registered, progress_tool, r);
        let execute_plan = ExecutePlanTool::new(
            Arc::clone(&self.session_manager),
            Arc::clone(&self.approval_flow),
        );
        try_register!(registry, registered, execute_plan, r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 2 plan tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
