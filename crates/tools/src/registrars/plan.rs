//! Plan tools registrar — registers ProgressTool.
//!
//! Registers the ProgressTool for plan execution step tracking.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use closeclaw_common::{PlanState, PlanStateWriter};

use crate::builtin::ProgressTool;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Plan tools registrar — registers tools from the plan domain.
///
/// Covers the `plan` group (1 tool): `ProgressTool`.
pub struct PlanToolsRegistrar {
    plan_state: Arc<Mutex<PlanState>>,
    writer: Option<Arc<dyn PlanStateWriter>>,
}

impl PlanToolsRegistrar {
    /// Create a new `PlanToolsRegistrar` with the given shared `PlanState`.
    pub fn new(plan_state: Arc<Mutex<PlanState>>) -> Self {
        Self {
            plan_state,
            writer: None,
        }
    }

    /// Create a new `PlanToolsRegistrar` with a [`PlanStateWriter`] for
    /// plan file synchronization.
    pub fn with_writer(
        plan_state: Arc<Mutex<PlanState>>,
        writer: Arc<dyn PlanStateWriter>,
    ) -> Self {
        Self {
            plan_state,
            writer: Some(writer),
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
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 1 plan tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
