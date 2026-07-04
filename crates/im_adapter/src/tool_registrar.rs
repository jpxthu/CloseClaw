//! ImAdapter tools registrar — Feishu tool group.
//!
//! Registers the 7 Feishu tools (im, calendar, task, bitable, doc, drive, sheet).

use async_trait::async_trait;

use closeclaw_tools::{Tool, ToolRegistrar, ToolRegistrarError};

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

    async fn register(
        &self,
        registry: &dyn closeclaw_common::tool_registry::ToolRegistry,
    ) -> Result<(), ToolRegistrarError> {
        let mut registered = 0usize;
        let r = self.name();
        closeclaw_tools::try_register!(registry, registered, FeishuImTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuCalendarTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuTaskTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuBitableTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuDocTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuDriveTool::new(), r);
        closeclaw_tools::try_register!(registry, registered, FeishuSheetTool::new(), r);
        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 7 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
