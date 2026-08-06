//! Skills tools registrar — registers a single tool provided by the caller.

use async_trait::async_trait;
use std::sync::Arc;

use closeclaw_common::tool_registry::{ToolBox, ToolRegistrar, ToolRegistrarError, ToolRegistry};

/// Skills tools registrar — registers a single tool from the skills domain.
///
/// The concrete tool instance (e.g. `SkillTool`) is provided by the
/// caller via [`SkillsToolsRegistrar::new`], so the skills crate does
/// not depend on the tools crate.
pub struct SkillsToolsRegistrar {
    tool: Arc<dyn closeclaw_common::Tool>,
}

impl SkillsToolsRegistrar {
    /// Create a new `SkillsToolsRegistrar` with the given tool instance.
    pub fn new(tool: Arc<dyn closeclaw_common::Tool>) -> Self {
        Self { tool }
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
        let boxed: Box<dyn std::any::Any + Send + Sync> = Box::new(ToolBox(Arc::clone(&self.tool)));
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
            })
    }
}
