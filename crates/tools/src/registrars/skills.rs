//! Skills tools registrar — registers SkillTool and SkillCreatorTool.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_gateway::SessionManager;
use closeclaw_skills::DiskSkillRegistry;

use crate::builtin::{SkillCreatorTool, SkillTool};
use crate::try_register;
use crate::{SpawnValidator, Tool};
use closeclaw_common::tool_registry::{ToolRegistrar, ToolRegistrarError};

/// Skills tools registrar — registers all tools from the skills domain.
///
/// Covers `skills` and `skill_creator` groups (2 tools):
/// `SkillTool`, `SkillCreatorTool`.
pub struct SkillsToolsRegistrar {
    disk_registry: Arc<DiskSkillRegistry>,
    spawn_validator: Arc<dyn SpawnValidator>,
    session_manager: Arc<SessionManager>,
}

impl SkillsToolsRegistrar {
    /// Create a new `SkillsToolsRegistrar` with the required dependencies.
    pub fn new(
        disk_registry: Arc<DiskSkillRegistry>,
        spawn_validator: Arc<dyn SpawnValidator>,
        session_manager: Arc<SessionManager>,
    ) -> Self {
        Self {
            disk_registry,
            spawn_validator,
            session_manager,
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
            SkillTool::new(
                self.disk_registry.clone(),
                self.spawn_validator.clone(),
                self.session_manager.clone(),
            ),
            r
        );
        try_register!(registry, registered, SkillCreatorTool::new(), r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 2 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
