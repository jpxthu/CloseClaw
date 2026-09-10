//! ImAdapter tools registrar — Feishu tool group.
//!
//! Registers the 22 Feishu sub-tools wrapped in [`LazyTool`] so
//! actual tool creation is deferred to first call.

use async_trait::async_trait;

use closeclaw_common::tool_trait::{Tool, ToolFlags};
use closeclaw_common::LazyTool;
use closeclaw_common::ToolMeta;
use closeclaw_tools::{ToolRegistrar, ToolRegistrarError};

use crate::platforms::feishu::tools::{
    FeishuBitableAppTableFieldTool, FeishuBitableAppTableRecordTool, FeishuBitableAppTableTool,
    FeishuBitableAppTableViewTool, FeishuBitableAppTool, FeishuCalendarCalendarTool,
    FeishuCalendarEventAttendeeTool, FeishuCalendarEventTool, FeishuCalendarFreebusyTool,
    FeishuDocCommentsTool, FeishuDocMediaTool, FeishuDriveFileTool, FeishuImUserGetMessagesTool,
    FeishuImUserGetThreadMessagesTool, FeishuImUserMessageTool, FeishuSearchDocWikiTool,
    FeishuSearchUserTool, FeishuSheetTool, FeishuTaskCommentTool, FeishuTaskSubtaskTool,
    FeishuTaskTaskTool, FeishuTaskTasklistTool,
};

/// Feishu / IM-Adapter tools registrar.
///
/// Covers 22 Feishu sub-tools (im ×4, calendar ×4, task ×4,
/// bitable ×5, doc ×3, drive ×1, sheet ×1).
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
        LazyTool::new(Box::new(|| Box::new(<$tool_type>::new())), $meta)
    }};
}

/// Register a single tool into the registry.
macro_rules! register {
    (
        $registry:expr,
        $registered:expr,
        $tool_type:ty,
        $name:expr,
        $group:expr,
        $summary:expr,
        $detail:expr,
        $r:expr
    ) => {{
        let tool = lazy_feishu_tool!(
            $tool_type,
            ToolMeta {
                name: $name.to_string(),
                group: $group.to_string(),
                summary: $summary.to_string(),
                detail: $detail.to_string(),
                input_schema: serde_json::json!({}),
                flags: feishu_flags(),
            }
        );
        closeclaw_tools::try_register!($registry, $registered, tool, $r);
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

        // ── feishu_im (4) ──────────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuImUserMessageTool,
            "feishu_im_user_message",
            "feishu_im",
            "Send or manage a Feishu IM message",
            "Send, recall, edit, and react to a single Feishu IM message.",
            r
        );
        register!(
            registry,
            registered,
            FeishuImUserGetMessagesTool,
            "feishu_im_user_get_messages",
            "feishu_im",
            "Get messages from a Feishu IM conversation",
            "Retrieve message history from a Feishu IM conversation.",
            r
        );
        register!(
            registry,
            registered,
            FeishuImUserGetThreadMessagesTool,
            "feishu_im_user_get_thread_messages",
            "feishu_im",
            "Get messages from a Feishu IM thread",
            "Retrieve message replies within a Feishu IM thread (topic).",
            r
        );
        register!(
            registry,
            registered,
            FeishuSearchUserTool,
            "feishu_search_user",
            "feishu_im",
            "Search for Feishu users",
            "Search for Feishu users by name or keyword.",
            r
        );

        // ── feishu_calendar (4) ────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuCalendarEventTool,
            "feishu_calendar_event",
            "feishu_calendar",
            "Manage a Feishu calendar event",
            "Create, update, delete, and query Feishu calendar events.",
            r
        );
        register!(
            registry,
            registered,
            FeishuCalendarEventAttendeeTool,
            "feishu_calendar_event_attendee",
            "feishu_calendar",
            "Manage attendees for a Feishu calendar event",
            "Add, remove, or update attendees for a Feishu calendar event.",
            r
        );
        register!(
            registry,
            registered,
            FeishuCalendarFreebusyTool,
            "feishu_calendar_freebusy",
            "feishu_calendar",
            "Query free/busy status for Feishu calendars",
            "Query free/busy time slots for one or more Feishu calendar users.",
            r
        );
        register!(
            registry,
            registered,
            FeishuCalendarCalendarTool,
            "feishu_calendar_calendar",
            "feishu_calendar",
            "Manage Feishu calendars",
            "List, create, update, and manage Feishu calendars.",
            r
        );

        // ── feishu_task (4) ────────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuTaskTaskTool,
            "feishu_task_task",
            "feishu_task",
            "Manage a Feishu task",
            "Create, update, complete, and query individual Feishu tasks.",
            r
        );
        register!(
            registry,
            registered,
            FeishuTaskTasklistTool,
            "feishu_task_tasklist",
            "feishu_task",
            "Manage Feishu task lists",
            "Create, update, and manage Feishu task lists.",
            r
        );
        register!(
            registry,
            registered,
            FeishuTaskCommentTool,
            "feishu_task_comment",
            "feishu_task",
            "Manage comments on a Feishu task",
            "Add, update, and retrieve comments on Feishu tasks.",
            r
        );
        register!(
            registry,
            registered,
            FeishuTaskSubtaskTool,
            "feishu_task_subtask",
            "feishu_task",
            "Manage subtasks of a Feishu task",
            "Create, update, and manage subtasks under a Feishu task.",
            r
        );

        // ── feishu_bitable (5) ─────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuBitableAppTool,
            "feishu_bitable_app",
            "feishu_bitable",
            "Manage a Feishu Bitable app",
            "Create, read, update, and manage Feishu Bitable apps.",
            r
        );
        register!(
            registry,
            registered,
            FeishuBitableAppTableTool,
            "feishu_bitable_app_table",
            "feishu_bitable",
            "Manage tables within a Feishu Bitable app",
            "Create, list, update, and delete tables within a Feishu Bitable app.",
            r
        );
        register!(
            registry,
            registered,
            FeishuBitableAppTableRecordTool,
            "feishu_bitable_app_table_record",
            "feishu_bitable",
            "Manage records in a Feishu Bitable table",
            "Create, read, update, and delete records in a Feishu Bitable table.",
            r
        );
        register!(
            registry,
            registered,
            FeishuBitableAppTableFieldTool,
            "feishu_bitable_app_table_field",
            "feishu_bitable",
            "Manage fields in a Feishu Bitable table",
            "List, create, update, and delete fields (columns) in a Feishu Bitable table.",
            r
        );
        register!(
            registry,
            registered,
            FeishuBitableAppTableViewTool,
            "feishu_bitable_app_table_view",
            "feishu_bitable",
            "Manage views in a Feishu Bitable table",
            "List, create, update, and configure views in a Feishu Bitable table.",
            r
        );

        // ── feishu_doc (3) ─────────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuDocCommentsTool,
            "feishu_doc_comments",
            "feishu_doc",
            "Manage comments on a Feishu document",
            "List, add, reply to, and resolve comments on Feishu documents.",
            r
        );
        register!(
            registry,
            registered,
            FeishuDocMediaTool,
            "feishu_doc_media",
            "feishu_doc",
            "Manage media in a Feishu document",
            "Upload, download, and manage images and file attachments in Feishu documents.",
            r
        );
        register!(
            registry,
            registered,
            FeishuSearchDocWikiTool,
            "feishu_search_doc_wiki",
            "feishu_doc",
            "Search Feishu documents and wiki pages",
            "Search for Feishu documents and wiki pages by keyword.",
            r
        );

        // ── feishu_drive (1) ───────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuDriveFileTool,
            "feishu_drive_file",
            "feishu_drive",
            "Manage files in Feishu Drive",
            "Upload, download, list, and manage files in Feishu Drive.",
            r
        );

        // ── feishu_sheet (1) ───────────────────────────────────────────
        register!(
            registry,
            registered,
            FeishuSheetTool,
            "feishu_sheet",
            "feishu_sheet",
            "Manage Feishu spreadsheets",
            "Read, write, and manage Feishu spreadsheets.",
            r
        );

        if registered == 0 {
            return Err(ToolRegistrarError::Internal(
                "all 22 tools failed to register".to_string(),
            ));
        }
        Ok(())
    }
}
