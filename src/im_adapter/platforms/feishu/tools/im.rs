//! Feishu IM tool group — messaging operations.
//!
//! Covers sending, recalling, editing, and reacting to messages.

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu IM message operations tool.
///
/// Provides message send, recall, edit, and reaction capabilities
/// for the Feishu messaging platform.
pub struct FeishuImTool;

impl FeishuImTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuImTool {
    fn name(&self) -> &str {
        "FeishuIm"
    }

    fn group(&self) -> &str {
        "feishu_im"
    }

    fn summary(&self) -> String {
        "Feishu IM message operations".to_string()
    }

    fn detail(&self) -> String {
        "Send, recall, edit, and react to Feishu messages. \
         Supports text and card message formats, thread replies, \
         and message deletion."
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
