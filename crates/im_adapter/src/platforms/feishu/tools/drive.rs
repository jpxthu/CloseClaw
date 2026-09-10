//! Feishu Drive sub-tool — cloud storage file operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_drive` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_DRV: &str = "[keywords: drive file folder upload download share]";

// ---------------------------------------------------------------------------
// feishu_drive_file
// ---------------------------------------------------------------------------

/// Manage files in Feishu Drive.
pub struct FeishuDriveFileTool;

impl Default for FeishuDriveFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDriveFileTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDriveFileTool {
    fn name(&self) -> &str {
        "feishu_drive_file"
    }

    fn group(&self) -> &str {
        "feishu_drive"
    }

    fn summary(&self) -> String {
        "Manage files in Feishu Drive".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DRV} Upload, download, list, and manage files \
             in Feishu Drive. Supports folder operations and \
             permission management."
        )
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({})
    }

    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolCallError> {
        Err(ToolCallError::NotImplemented)
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_deferred_by_default: true,
            ..ToolFlags::default()
        }
    }
}
