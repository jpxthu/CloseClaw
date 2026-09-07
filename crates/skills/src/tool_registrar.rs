//! Skills tools registrar — registers tools provided by the caller.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::tool_registry::{ToolBox, ToolRegistrar, ToolRegistrarError, ToolRegistry};

/// Skills tools registrar — registers tools from the skills domain.
///
/// Concrete tool instances (e.g. `SkillTool`, `SkillCreatorTool`) are
/// provided by the caller via [`SkillsToolsRegistrar::new`], so the
/// skills crate does not depend on the tools crate.
pub struct SkillsToolsRegistrar {
    tools: Vec<Arc<dyn closeclaw_common::Tool>>,
}

impl SkillsToolsRegistrar {
    /// Create a new `SkillsToolsRegistrar` with the given tool instances.
    pub fn new(tools: Vec<Arc<dyn closeclaw_common::Tool>>) -> Self {
        Self { tools }
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

    async fn register(&self, registry: &dyn ToolRegistry) -> Result<(), ToolRegistrarError> {
        let registrar_name = self.name();
        for tool in &self.tools {
            let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(ToolBox(Arc::clone(tool)));
            registry
                .register_any(boxed, registrar_name)
                .await
                .map_err(|e| match e {
                    closeclaw_common::tool_registry::RegistryError::Conflict {
                        tool,
                        registrar,
                        attempting,
                    } => ToolRegistrarError::Conflict {
                        tool,
                        registrar,
                        attempting,
                    },
                    closeclaw_common::tool_registry::RegistryError::AlreadyRegistered(name) => {
                        ToolRegistrarError::Conflict {
                            tool: name,
                            registrar: String::new(),
                            attempting: registrar_name.to_string(),
                        }
                    }
                    other => ToolRegistrarError::Internal(other.to_string()),
                })?;
        }
        Ok(())
    }
}
