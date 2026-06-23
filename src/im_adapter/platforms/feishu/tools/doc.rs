//! Feishu Doc tool group — document operations.
//!
//! Covers creating, reading, updating, and managing Feishu documents.

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu Document tool.
///
/// Provides document create, read, update, and management
/// capabilities for the Feishu document platform.
pub struct FeishuDocTool;

impl Default for FeishuDocTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocTool {
    fn name(&self) -> &str {
        "FeishuDoc"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Feishu document operations".to_string()
    }

    fn detail(&self) -> String {
        "Create, read, update, and manage Feishu documents. \
         Supports content editing, permission management, \
         and document metadata."
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
