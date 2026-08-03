//! Skills tools registrar — registers SkillTool.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_skills::{BuiltinSkillRegistry, DiskSkillRegistry};

use crate::builtin::SkillTool;
use crate::try_register;
use crate::Tool;
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Skills tools registrar — registers all tools from the skills domain.
///
/// Covers `skills` group (1 tool):
/// `SkillTool`.
pub struct SkillsToolsRegistrar {
    disk_registry: Arc<DiskSkillRegistry>,
    builtin_registry: Arc<BuiltinSkillRegistry>,
}

impl SkillsToolsRegistrar {
    /// Create a new `SkillsToolsRegistrar` with the required dependencies.
    pub fn new(
        disk_registry: Arc<DiskSkillRegistry>,
        builtin_registry: Arc<BuiltinSkillRegistry>,
    ) -> Self {
        Self {
            disk_registry,
            builtin_registry,
        }
    }
}

#[async_trait]
impl ToolRegistrar for SkillsToolsRegistrar {
    fn name(&self) -> &str {
        "SkillsToolsRegistrar"
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
        try_register!(
            registry,
            registered,
            SkillTool::new(self.disk_registry.clone(), self.builtin_registry.clone(),),
            r
        );
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 1 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
