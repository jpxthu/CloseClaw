//! ImAdapter tools registrar — Feishu tool group.
//!
//! Registers the 7 Feishu tools (im, calendar, task, bitable, doc, drive, sheet)
//! wrapped in [`LazyTool`] so actual tool creation is deferred to first call.

use async_trait::async_trait;

use closeclaw_common::LazyTool;
use closeclaw_common::ToolMeta;
use closeclaw_common::tool_trait::{Tool, ToolFlags};
use closeclaw_tools::{ToolRegistrar, ToolRegistrarError};

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

/// Default flags for all Feishu tools.
fn feishu_flags() -> ToolFlags {
    ToolFlags {
        is_deferred_by_default: true,
        ..ToolFlags::default()
    }
}

/// Build a [`LazyTool`] for a given Feishu tool type.
///
/// Metadata is specified as literal `ToolMeta` — no tool instance is
/// created during registration, preserving the deferred-init contract.
macro_rules! lazy_feishu_tool {
    ($tool_type:ty, $meta:expr) => {{
        LazyTool::new(
            Box::new(|| Box::new(<$tool_type>::new())),
            $meta,
        )
    }};
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
        let flags = feishu_flags();

        let feishu_im = lazy_feishu_tool!(FeishuImTool, ToolMeta {
            name: "FeishuIm".to_string(),
            group: "feishu_im".to_string(),
            summary: "Feishu IM message operations".to_string(),
            detail: "Send, recall, edit, and react to Feishu messages. \
                     Supports text and card message formats, thread replies, \
                     and message deletion."
                .to_string(),
            input_schema: serde_json::json!({}),
            flags,
        });
        closeclaw_tools::try_register!(
            registry, registered, feishu_im, r
        );

        let feishu_calendar = lazy_feishu_tool!(
            FeishuCalendarTool,
            ToolMeta {
                name: "FeishuCalendar".to_string(),
                group: "feishu_calendar".to_string(),
                summary: "Feishu calendar management".to_string(),
                detail: "Create, update, delete, and query Feishu calendar \
                         events. Supports attendee management, recurring \
                         events, and calendar list operations."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_calendar, r
        );

        let feishu_task = lazy_feishu_tool!(
            FeishuTaskTool,
            ToolMeta {
                name: "FeishuTask".to_string(),
                group: "feishu_task".to_string(),
                summary: "Feishu task management".to_string(),
                detail: "Create, update, complete, and query Feishu tasks. \
                         Supports task lists, reminders, and collaborator \
                         management."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_task, r
        );

        let feishu_bitable = lazy_feishu_tool!(
            FeishuBitableTool,
            ToolMeta {
                name: "FeishuBitable".to_string(),
                group: "feishu_bitable".to_string(),
                summary: "Feishu Bitable table operations".to_string(),
                detail: "Create, read, update, and delete records in Feishu \
                         Bitable. Supports table and field management, view \
                         configuration, and batch operations."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_bitable, r
        );

        let feishu_doc = lazy_feishu_tool!(
            FeishuDocTool,
            ToolMeta {
                name: "FeishuDoc".to_string(),
                group: "feishu_doc".to_string(),
                summary: "Feishu document operations".to_string(),
                detail: "Create, read, update, and manage Feishu documents. \
                         Supports content editing, permission management, \
                         and document metadata."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_doc, r
        );

        let feishu_drive = lazy_feishu_tool!(
            FeishuDriveTool,
            ToolMeta {
                name: "FeishuDrive".to_string(),
                group: "feishu_drive".to_string(),
                summary: "Feishu Drive file operations".to_string(),
                detail: "Upload, download, list, and manage files in Feishu \
                         Drive. Supports folder operations, file sharing, \
                         and permission management."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_drive, r
        );

        let feishu_sheet = lazy_feishu_tool!(
            FeishuSheetTool,
            ToolMeta {
                name: "FeishuSheet".to_string(),
                group: "feishu_sheet".to_string(),
                summary: "Feishu spreadsheet operations".to_string(),
                detail: "Read, write, and manage Feishu spreadsheets. \
                         Supports cell operations, sheet management, \
                         and data range manipulation."
                    .to_string(),
                input_schema: serde_json::json!({}),
                flags,
            }
        );
        closeclaw_tools::try_register!(
            registry, registered, feishu_sheet, r
        );

        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 7 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
