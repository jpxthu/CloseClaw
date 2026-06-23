//! Feishu Sheet tool group — spreadsheet operations.
//!
//! Covers reading, writing, and managing Feishu spreadsheets.

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu Sheet (spreadsheet) tool.
///
/// Provides spreadsheet read, write, and management
/// capabilities for the Feishu Sheet platform.
pub struct FeishuSheetTool;

impl FeishuSheetTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuSheetTool {
    fn name(&self) -> &str {
        "FeishuSheet"
    }

    fn group(&self) -> &str {
        "feishu_sheet"
    }

    fn summary(&self) -> String {
        "Feishu spreadsheet operations".to_string()
    }

    fn detail(&self) -> String {
        "Read, write, and manage Feishu spreadsheets. \
         Supports cell operations, sheet management, \
         and data range manipulation."
            .to_string()
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
