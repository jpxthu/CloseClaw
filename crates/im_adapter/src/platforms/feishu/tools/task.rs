//! Feishu Task tool group — task management operations.
//!
//! Covers creating, updating, completing, and querying tasks.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

/// Feishu Task management tool.
///
/// Provides task create, update, complete, and query
/// capabilities for the Feishu task platform.
pub struct FeishuTaskTool;

impl Default for FeishuTaskTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuTaskTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuTaskTool {
    fn name(&self) -> &str {
        "FeishuTask"
    }

    fn group(&self) -> &str {
        "feishu_task"
    }

    fn summary(&self) -> String {
        "Feishu task management".to_string()
    }

    fn detail(&self) -> String {
        "Create, update, complete, and query Feishu tasks. \
         Supports task lists, reminders, and collaborator management."
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
