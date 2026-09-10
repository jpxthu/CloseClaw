//! Feishu IM sub-tools — message and user operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_im` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

/// Common keyword prefix for IM tools.
const KW_IM: &str = "[keywords: message chat send recall edit react im feishu]";

// ---------------------------------------------------------------------------
// feishu_im_user_message
// ---------------------------------------------------------------------------

/// Send or manage a single Feishu IM message.
pub struct FeishuImUserMessageTool;

impl Default for FeishuImUserMessageTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuImUserMessageTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuImUserMessageTool {
    fn name(&self) -> &str {
        "feishu_im_user_message"
    }

    fn group(&self) -> &str {
        "feishu_im"
    }

    fn summary(&self) -> String {
        "Send or manage a Feishu IM message".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_IM} Send, recall, edit, and react to a single \
             Feishu IM message."
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
// feishu_im_user_get_messages
// ---------------------------------------------------------------------------

/// Retrieve messages from a Feishu IM conversation.
pub struct FeishuImUserGetMessagesTool;

impl Default for FeishuImUserGetMessagesTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuImUserGetMessagesTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuImUserGetMessagesTool {
    fn name(&self) -> &str {
        "feishu_im_user_get_messages"
    }

    fn group(&self) -> &str {
        "feishu_im"
    }

    fn summary(&self) -> String {
        "Get messages from a Feishu IM conversation".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_IM} Retrieve message history from a Feishu \
             IM conversation."
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
// feishu_im_user_get_thread_messages
// ---------------------------------------------------------------------------

/// Retrieve messages from a Feishu IM thread.
pub struct FeishuImUserGetThreadMessagesTool;

impl Default for FeishuImUserGetThreadMessagesTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuImUserGetThreadMessagesTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuImUserGetThreadMessagesTool {
    fn name(&self) -> &str {
        "feishu_im_user_get_thread_messages"
    }

    fn group(&self) -> &str {
        "feishu_im"
    }

    fn summary(&self) -> String {
        "Get messages from a Feishu IM thread".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_IM} Retrieve message replies within a Feishu \
             IM thread (topic)."
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
// feishu_search_user
// ---------------------------------------------------------------------------

/// Search for Feishu users by keyword.
pub struct FeishuSearchUserTool;

impl Default for FeishuSearchUserTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuSearchUserTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuSearchUserTool {
    fn name(&self) -> &str {
        "feishu_search_user"
    }

    fn group(&self) -> &str {
        "feishu_im"
    }

    fn summary(&self) -> String {
        "Search for Feishu users".to_string()
    }

    fn detail(&self) -> String {
        format!("{KW_IM} Search for Feishu users by name or keyword.")
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
