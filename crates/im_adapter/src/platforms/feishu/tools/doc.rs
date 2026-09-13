//! Feishu Doc sub-tools — document and wiki operations.
//!
//! Each tool corresponds to a single row in the tools README
//! "各模块注册的工具一览" table for the `feishu_doc` group.

use async_trait::async_trait;
use closeclaw_tools::{Tool, ToolCallError, ToolContext, ToolFlags, ToolResult};
use serde_json::Value;

const KW_DOC: &str = "[keywords: doc document create edit content block]";

// ---------------------------------------------------------------------------
// feishu_doc_create
// ---------------------------------------------------------------------------

/// Create a new Feishu document.
pub struct FeishuDocCreateTool;

impl Default for FeishuDocCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocCreateTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocCreateTool {
    fn name(&self) -> &str {
        "feishu_doc_create"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Create a new Feishu document".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} Create a new Feishu document with title and \
             optional initial content blocks."
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
// feishu_doc_edit
// ---------------------------------------------------------------------------

/// Edit an existing Feishu document.
pub struct FeishuDocEditTool;

impl Default for FeishuDocEditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocEditTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocEditTool {
    fn name(&self) -> &str {
        "feishu_doc_edit"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Edit an existing Feishu document".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} Edit content blocks in an existing Feishu \
             document (insert, update, delete blocks)."
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
// feishu_doc_read
// ---------------------------------------------------------------------------

/// Read content from a Feishu document.
pub struct FeishuDocReadTool;

impl Default for FeishuDocReadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocReadTool {
    fn name(&self) -> &str {
        "feishu_doc_read"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Read content from a Feishu document".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} Read content blocks from a Feishu document \
             by document ID."
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
// feishu_doc_comments
// ---------------------------------------------------------------------------

/// Manage comments on a Feishu document.
pub struct FeishuDocCommentsTool;

impl Default for FeishuDocCommentsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocCommentsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocCommentsTool {
    fn name(&self) -> &str {
        "feishu_doc_comments"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Manage comments on a Feishu document".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} List, add, reply to, and resolve comments \
             on Feishu documents."
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
// feishu_doc_media
// ---------------------------------------------------------------------------

/// Manage media (images, files) in a Feishu document.
pub struct FeishuDocMediaTool;

impl Default for FeishuDocMediaTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuDocMediaTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuDocMediaTool {
    fn name(&self) -> &str {
        "feishu_doc_media"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Manage media in a Feishu document".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} Upload, download, and manage images and \
             file attachments in Feishu documents."
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
// feishu_search_doc_wiki
// ---------------------------------------------------------------------------

/// Search for Feishu documents and wiki pages.
pub struct FeishuSearchDocWikiTool;

impl Default for FeishuSearchDocWikiTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FeishuSearchDocWikiTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FeishuSearchDocWikiTool {
    fn name(&self) -> &str {
        "feishu_search_doc_wiki"
    }

    fn group(&self) -> &str {
        "feishu_doc"
    }

    fn summary(&self) -> String {
        "Search Feishu documents and wiki pages".to_string()
    }

    fn detail(&self) -> String {
        format!(
            "{KW_DOC} Search for Feishu documents and wiki \
             pages by keyword."
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
