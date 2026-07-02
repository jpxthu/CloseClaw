//! ImAdapter tools registrar — Feishu tool group.
//!
//! Registers the 7 Feishu tools (im, calendar, task, bitable, doc, drive, sheet).

use async_trait::async_trait;

use closeclaw_tools::registrar::{ToolRegistrar, ToolRegistrarError};
use closeclaw_tools::ToolRegistry;

use crate::platforms::feishu::tools::{
    FeishuBitableTool, FeishuCalendarTool, FeishuDocTool, FeishuDriveTool, FeishuImTool,
    FeishuSheetTool, FeishuTaskTool,
};

/// Feishu / IM-Adapter tools registrar.
///
/// Covers the `feishu_im` and related Feishu tool groups (7 tools).
pub struct ImAdapterToolsRegistrar;

impl ImAdapterToolsRegistrar {
    /// Create a new `ImAdapterToolsRegistrar`.
    pub fn new() -> Self {
        Self
    }
}

impl Default for ImAdapterToolsRegistrar {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolRegistrar for ImAdapterToolsRegistrar {
    fn name(&self) -> &str {
        "ImAdapterToolsRegistrar"
    }

    fn priority(&self) -> u32 {
        4
    }

    async fn register(&self, registry: &ToolRegistry) -> Result<(), ToolRegistrarError> {
        register_tool(registry, FeishuImTool::new()).await?;
        register_tool(registry, FeishuCalendarTool::new()).await?;
        register_tool(registry, FeishuTaskTool::new()).await?;
        register_tool(registry, FeishuBitableTool::new()).await?;
        register_tool(registry, FeishuDocTool::new()).await?;
        register_tool(registry, FeishuDriveTool::new()).await?;
        register_tool(registry, FeishuSheetTool::new()).await?;

        Ok(())
    }
}

/// Helper: register a single tool, converting `ToolError` into `ToolRegistrarError`.
async fn register_tool(
    registry: &ToolRegistry,
    tool: impl closeclaw_tools::Tool + 'static,
) -> Result<(), ToolRegistrarError> {
    registry.register(tool).await.map_err(|e| match e {
        closeclaw_tools::ToolError::AlreadyRegistered(name) => ToolRegistrarError::Conflict {
            tool: name,
            registrar: "ImAdapterToolsRegistrar".to_string(),
        },
        other => ToolRegistrarError::Internal(other.to_string()),
    })
}
