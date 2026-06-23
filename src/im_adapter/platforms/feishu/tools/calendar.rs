//! Feishu Calendar tool group — calendar management operations.
//!
//! Covers creating, updating, deleting, and querying calendar events.

use crate::tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Feishu Calendar management tool.
///
/// Provides calendar event create, update, delete, and query
/// capabilities for the Feishu calendar platform.
pub struct FeishuCalendarTool;

impl FeishuCalendarTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuCalendarTool {
    fn name(&self) -> &str {
        "FeishuCalendar"
    }

    fn group(&self) -> &str {
        "feishu_calendar"
    }

    fn summary(&self) -> String {
        "Feishu calendar management".to_string()
    }

    fn detail(&self) -> String {
        "Create, update, delete, and query Feishu calendar events. \
         Supports attendee management, recurring events, and \
         calendar list operations."
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
