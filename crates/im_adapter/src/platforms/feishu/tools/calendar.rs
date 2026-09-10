//! Feishu Calendar sub-tools — event and schedule operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_calendar` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_CAL: &str = "[keywords: calendar event schedule meeting create update delete query]";

// ---------------------------------------------------------------------------
// feishu_calendar_event
// ---------------------------------------------------------------------------

/// Create, update, or delete a Feishu calendar event.
pub struct FeishuCalendarEventTool;

impl Default for FeishuCalendarEventTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuCalendarEventTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuCalendarEventTool {
    fn name(&self) -> &str {
        "feishu_calendar_event"
    }

    fn group(&self) -> &str {
        "feishu_calendar"
    }

    fn summary(&self) -> String {
        "Manage a Feishu calendar event".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_CAL} Create, update, delete, and query \
             Feishu calendar events."
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

// ---------------------------------------------------------------------------
// feishu_calendar_event_attendee
// ---------------------------------------------------------------------------

/// Manage attendees for a Feishu calendar event.
pub struct FeishuCalendarEventAttendeeTool;

impl Default for FeishuCalendarEventAttendeeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuCalendarEventAttendeeTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuCalendarEventAttendeeTool {
    fn name(&self) -> &str {
        "feishu_calendar_event_attendee"
    }

    fn group(&self) -> &str {
        "feishu_calendar"
    }

    fn summary(&self) -> String {
        "Manage attendees for a Feishu calendar event".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_CAL} Add, remove, or update attendees for \
             a Feishu calendar event."
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

// ---------------------------------------------------------------------------
// feishu_calendar_freebusy
// ---------------------------------------------------------------------------

/// Query free/busy status for Feishu calendars.
pub struct FeishuCalendarFreebusyTool;

impl Default for FeishuCalendarFreebusyTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuCalendarFreebusyTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuCalendarFreebusyTool {
    fn name(&self) -> &str {
        "feishu_calendar_freebusy"
    }

    fn group(&self) -> &str {
        "feishu_calendar"
    }

    fn summary(&self) -> String {
        "Query free/busy status for Feishu calendars".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_CAL} Query free/busy time slots for one or \
             more Feishu calendar users."
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

// ---------------------------------------------------------------------------
// feishu_calendar_calendar
// ---------------------------------------------------------------------------

/// Manage Feishu calendars (list, create, update).
pub struct FeishuCalendarCalendarTool;

impl Default for FeishuCalendarCalendarTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuCalendarCalendarTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuCalendarCalendarTool {
    fn name(&self) -> &str {
        "feishu_calendar_calendar"
    }

    fn group(&self) -> &str {
        "feishu_calendar"
    }

    fn summary(&self) -> String {
        "Manage Feishu calendars".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_CAL} List, create, update, and manage \
             Feishu calendars."
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
