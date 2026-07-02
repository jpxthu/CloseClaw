//! Skills tools registrar — registers SkillTool and SkillCreatorTool.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_gateway::SessionManager;
use closeclaw_skills::DiskSkillRegistry;

use crate::builtin::{SkillCreatorTool, SkillTool};
use crate::registrar::{ToolRegistrar, ToolRegistrarError};
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
        )
        .await?;
        register_tool(registry, SkillCreatorTool::new()).await?;

        Ok(())
    }
}

/// Helper: register a single tool, converting `ToolError` into
/// `ToolRegistrarError`.
async fn register_tool(
    registry: &ToolRegistry,
    tool: impl crate::Tool + 'static,
) -> Result<(), ToolRegistrarError> {
    registry.register(tool).await.map_err(|e| match e {
        crate::ToolError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
            tool: name,
            registrar: "SkillsToolsRegistrar".to_string(),
        },
        other => ToolRegistrarError::Internal(other.to_string()),
    })
}
