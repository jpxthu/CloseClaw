//! Feishu Sheet sub-tool — spreadsheet operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_sheet` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_SHT: &str = "[keywords: sheet spreadsheet cell row column formula]";

// ---------------------------------------------------------------------------
// feishu_sheet
// ---------------------------------------------------------------------------

/// Read, write, and manage Feishu spreadsheets.
pub struct FeishuSheetTool;

impl Default for FeishuSheetTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuSheetTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuSheetTool {
    fn name(&self) -> &str {
        "feishu_sheet"
    }

    fn group(&self) -> &str {
        "feishu_sheet"
    }

    fn summary(&self) -> String {
        "Manage Feishu spreadsheets".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_SHT} Read, write, and manage Feishu spreadsheets. \
             Supports cell operations, sheet management, \
             and data range manipulation."
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
