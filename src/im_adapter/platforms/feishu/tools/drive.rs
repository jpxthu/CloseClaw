//! Feishu Drive tool group — cloud storage operations.
//!
//! Covers file upload, download, listing, and management in
//! Feishu Drive.

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu Drive (cloud storage) tool.
///
/// Provides file upload, download, listing, and management
/// capabilities for the Feishu Drive platform.
pub struct FeishuDriveTool;

impl FeishuDriveTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDriveTool {
    fn name(&self) -> &str {
        "FeishuDrive"
    }

    fn group(&self) -> &str {
        "feishu_drive"
    }

    fn summary(&self) -> String {
        "Feishu Drive file operations".to_string()
    }

    fn detail(&self) -> String {
        "Upload, download, list, and manage files in Feishu Drive. \
         Supports folder operations, file sharing, and \
         permission management."
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
