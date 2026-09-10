//! Feishu Task sub-tools — task management operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_task` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_TASK: &str = "[keywords: task todo create update complete assign]";

// ---------------------------------------------------------------------------
// feishu_task_task
// ---------------------------------------------------------------------------

/// Create, update, or complete a Feishu task.
pub struct FeishuTaskTaskTool;

impl Default for FeishuTaskTaskTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuTaskTaskTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuTaskTaskTool {
    fn name(&self) -> &str {
        "feishu_task_task"
    }

    fn group(&self) -> &str {
        "feishu_task"
    }

    fn summary(&self) -> String {
        "Manage a Feishu task".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_TASK} Create, update, complete, and query \
             individual Feishu tasks."
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
// feishu_task_tasklist
// ---------------------------------------------------------------------------

/// Manage Feishu task lists.
pub struct FeishuTaskTasklistTool;

impl Default for FeishuTaskTasklistTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuTaskTasklistTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuTaskTasklistTool {
    fn name(&self) -> &str {
        "feishu_task_tasklist"
    }

    fn group(&self) -> &str {
        "feishu_task"
    }

    fn summary(&self) -> String {
        "Manage Feishu task lists".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_TASK} Create, update, and manage Feishu \
             task lists."
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
// feishu_task_comment
// ---------------------------------------------------------------------------

/// Manage comments on a Feishu task.
pub struct FeishuTaskCommentTool;

impl Default for FeishuTaskCommentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuTaskCommentTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuTaskCommentTool {
    fn name(&self) -> &str {
        "feishu_task_comment"
    }

    fn group(&self) -> &str {
        "feishu_task"
    }

    fn summary(&self) -> String {
        "Manage comments on a Feishu task".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_TASK} Add, update, and retrieve comments on \
             Feishu tasks."
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
// feishu_task_subtask
// ---------------------------------------------------------------------------

/// Manage subtasks of a Feishu task.
pub struct FeishuTaskSubtaskTool;

impl Default for FeishuTaskSubtaskTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuTaskSubtaskTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuTaskSubtaskTool {
    fn name(&self) -> &str {
        "feishu_task_subtask"
    }

    fn group(&self) -> &str {
        "feishu_task"
    }

    fn summary(&self) -> String {
        "Manage subtasks of a Feishu task".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_TASK} Create, update, and manage subtasks \
             under a Feishu task."
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
