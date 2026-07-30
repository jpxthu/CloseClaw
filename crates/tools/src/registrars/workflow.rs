//! Workflow tools registrar.
//!
//! Registers the four workflow tools (`workflow_start`, `workflow_verify`,
//! `workflow_jump`, `workflow_blocked`) into the [`ToolRegistry`].

use async_trait::async_trait;

use crate::builtin::{
    WorkflowBlockedTool, WorkflowJumpTool, WorkflowStartTool, WorkflowVerifyTool,
};
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Workflow tools registrar — registers all tools from the workflow domain.
///
/// Covers the four workflow tools: `workflow_start`, `workflow_verify`,
/// `workflow_jump`, and `workflow_blocked`.
pub struct WorkflowToolsRegistrar;

impl WorkflowToolsRegistrar {
    /// Create a new `WorkflowToolsRegistrar`.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolRegistrar for WorkflowToolsRegistrar {
    fn name(&self) -> &str {
        "WorkflowToolsRegistrar"
    }

    fn priority(&self) -> u32 {
        10
    }

    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        try_register!(registry, registered, WorkflowStartTool, r);
        try_register!(registry, registered, WorkflowVerifyTool, r);
        try_register!(registry, registered, WorkflowJumpTool, r);
        try_register!(registry, registered, WorkflowBlockedTool, r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 4 workflow tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
