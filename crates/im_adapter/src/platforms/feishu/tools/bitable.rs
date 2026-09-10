//! Feishu Bitable sub-tools — multi-dimensional table operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_bitable` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_BT: &str = "[keywords: bitable database table record row field]";

// ---------------------------------------------------------------------------
// feishu_bitable_app
// ---------------------------------------------------------------------------

/// Manage a Feishu Bitable app.
pub struct FeishuBitableAppTool;

impl Default for FeishuBitableAppTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuBitableAppTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableAppTool {
    fn name(&self) -> &str {
        "feishu_bitable_app"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Manage a Feishu Bitable app".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_BT} Create, read, update, and manage \
             Feishu Bitable apps."
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
// feishu_bitable_app_table
// ---------------------------------------------------------------------------

/// Manage tables within a Feishu Bitable app.
pub struct FeishuBitableAppTableTool;

impl Default for FeishuBitableAppTableTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuBitableAppTableTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableAppTableTool {
    fn name(&self) -> &str {
        "feishu_bitable_app_table"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Manage tables within a Feishu Bitable app".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_BT} Create, list, update, and delete tables \
             within a Feishu Bitable app."
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
// feishu_bitable_app_table_record
// ---------------------------------------------------------------------------

/// Manage records in a Feishu Bitable table.
pub struct FeishuBitableAppTableRecordTool;

impl Default for FeishuBitableAppTableRecordTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuBitableAppTableRecordTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableAppTableRecordTool {
    fn name(&self) -> &str {
        "feishu_bitable_app_table_record"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Manage records in a Feishu Bitable table".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_BT} Create, read, update, and delete records \
             in a Feishu Bitable table."
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
// feishu_bitable_app_table_field
// ---------------------------------------------------------------------------

/// Manage fields in a Feishu Bitable table.
pub struct FeishuBitableAppTableFieldTool;

impl Default for FeishuBitableAppTableFieldTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuBitableAppTableFieldTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableAppTableFieldTool {
    fn name(&self) -> &str {
        "feishu_bitable_app_table_field"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Manage fields in a Feishu Bitable table".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_BT} List, create, update, and delete fields \
             (columns) in a Feishu Bitable table."
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
// feishu_bitable_app_table_view
// ---------------------------------------------------------------------------

/// Manage views in a Feishu Bitable table.
pub struct FeishuBitableAppTableViewTool;

impl Default for FeishuBitableAppTableViewTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuBitableAppTableViewTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuBitableAppTableViewTool {
    fn name(&self) -> &str {
        "feishu_bitable_app_table_view"
    }

    fn group(&self) -> &str {
        "feishu_bitable"
    }

    fn summary(&self) -> String {
        "Manage views in a Feishu Bitable table".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_BT} List, create, update, and configure views \
             in a Feishu Bitable table."
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
