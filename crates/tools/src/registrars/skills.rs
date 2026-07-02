//! Skills tools registrar — registers SkillTool and SkillCreatorTool.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_gateway::SessionManager;
use closeclaw_skills::DiskSkillRegistry;

use crate::builtin::{SkillCreatorTool, SkillTool};
use crate::registrar::{register_tool, ToolRegistrar, ToolRegistrarError};
use crate::{SpawnValidator, ToolRegistry};

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

    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        register_tool(
            registry,
            SkillTool::new(
                self.disk_registry.clone(),
                self.spawn_validator.clone(),
                self.session_manager.clone(),
            ),
            "SkillsToolsRegistrar",
        )
        .await?;
        register_tool(registry, SkillCreatorTool::new(), "SkillsToolsRegistrar").await?;

        Ok(())
    }
}
