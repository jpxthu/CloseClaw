//! Feishu Bitable tool group — multi-dimensional table operations.
//!
//! Covers creating, reading, updating, and deleting records in
//! Feishu Bitable (multi-dimensional tables).

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu Bitable (multi-dimensional table) tool.
///
/// Provides record CRUD, table management, and field operations
/// for the Feishu Bitable platform.
pub struct FeishuBitableTool;

impl FeishuBitableTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableTool {
    fn name(&self) -> &str {
        "FeishuBitable"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Feishu Bitable table operations".to_string()
    }

    fn detail(&self) -> String {
        "Create, read, update, and delete records in Feishu Bitable. \
         Supports table and field management, view configuration, \
         and batch operations."
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
