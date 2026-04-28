//! Built-in tools — file operations (Tool trait implementation).
//!
//! Each tool is an independent [`Tool`] implementation, completely separate
//! from the [`crate::skills`] module.

use crate::tools::{Tool, ToolContext, ToolError, ToolFlags};

use serde_json::Value;
use std::path::Path;

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

fn workdir_path(ctx: &ToolContext, path: &str) -> std::path::PathBuf {
    match ctx.workdir {
        Some(ref wd) => std::path::Path::new(&wd.path).join(path),
        None => std::path::Path::new(path).to_path_buf(),
    }
}

// ---------------------------------------------------------------------------
// ReadTool
// ---------------------------------------------------------------------------

pub struct ReadTool;

impl ReadTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Read file contents".to_string()
    }

    fn detail(&self) -> String {
        "Read the full contents of a file given its path.\
         Returns the text content as a JSON object with key `content`.\
         Fails if the path does not exist or is not a readable file."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                }
            },
            "required": ["path"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// WriteTool
// ---------------------------------------------------------------------------

pub struct WriteTool;

impl WriteTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Write content to a file".to_string()
    }

    fn detail(&self) -> String {
        "Write text content to a file, creating it or overwriting it.\
         Takes `path` (string) and `content` (string).\
         Parent directories are created automatically.\
         Destructive: will overwrite existing files without warning."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                },
                "content": {
                    "type": "string",
                    "description": "Text content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: false,
            is_destructive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// EditTool
// ---------------------------------------------------------------------------

pub struct EditTool;

impl EditTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Apply a targeted edit to a file".to_string()
    }

    fn detail(&self) -> String {
        "Apply a targeted edit to an existing file using exact text replacement.\
         Takes `path`, `oldText` (exact string to replace), and `newText` (replacement).\
         Only one replacement is performed per call.\
         Fails if `oldText` is not found verbatim in the file.\
         Destructive: modifies the file in place."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or workdir-relative file path"
                },
                "oldText": {
                    "type": "string",
                    "description": "Exact text to search for and replace"
                },
                "newText": {
                    "type": "string",
                    "description": "Replacement text"
                }
            },
            "required": ["path", "oldText", "newText"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: false,
            is_destructive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// GrepTool
// ---------------------------------------------------------------------------

pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "Search for text patterns in files".to_string()
    }

    fn detail(&self) -> String {
        "Recursively search for lines matching a pattern in files.\
         Takes `pattern` (string or regex), `path` (directory, default \".\"),\
         and optional `is_regex` (bool, default false).\
         Returns a JSON array of `{file, line_number, line}` objects.\
         Read-only: does not modify any file."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern or regex"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default .)"
                },
                "is_regex": {
                    "type": "boolean",
                    "description": "Treat pattern as regex (default false)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            is_expensive: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// LsTool
// ---------------------------------------------------------------------------

pub struct LsTool;

impl LsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for LsTool {
    fn name(&self) -> &str {
        "Ls"
    }

    fn group(&self) -> &str {
        "file_ops"
    }

    fn summary(&self) -> String {
        "List directory entries".to_string()
    }

    fn detail(&self) -> String {
        "List entries in a directory.\
         Takes optional `path` (directory, default \".\").\
         Returns a JSON array of entry names.\
         Read-only: does not modify any file or directory."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory to list (default .)"
                }
            },
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test".to_string(),
            workdir: None,
        }
    }

    #[test]
    fn test_read_name_group_summary() {
        let tool = ReadTool::new();
        assert_eq!(tool.name(), "Read");
        assert_eq!(tool.group(), "file_ops");
        assert!(tool.summary().len() <= 50);
        assert!(tool.flags().is_read_only);
        assert!(!tool.flags().is_destructive);
    }

    #[test]
    fn test_write_name_group_summary() {
        let tool = WriteTool::new();
        assert_eq!(tool.name(), "Write");
        assert_eq!(tool.group(), "file_ops");
        assert!(tool.summary().len() <= 50);
        assert!(tool.flags().is_destructive);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_edit_name_group_summary() {
        let tool = EditTool::new();
        assert_eq!(tool.name(), "Edit");
        assert_eq!(tool.group(), "file_ops");
        assert!(tool.summary().len() <= 50);
        assert!(tool.flags().is_destructive);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_grep_name_group_summary() {
        let tool = GrepTool::new();
        assert_eq!(tool.name(), "Grep");
        assert_eq!(tool.group(), "file_ops");
        assert!(tool.summary().len() <= 50);
        assert!(tool.flags().is_read_only);
    }

    #[test]
    fn test_ls_name_group_summary() {
        let tool = LsTool::new();
        assert_eq!(tool.name(), "Ls");
        assert_eq!(tool.group(), "file_ops");
        assert!(tool.summary().len() <= 50);
        assert!(tool.flags().is_read_only);
    }

    #[test]
    fn test_read_input_schema_has_path() {
        let tool = ReadTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("path"));
    }

    #[test]
    fn test_write_input_schema_has_path_and_content() {
        let tool = WriteTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("content"));
    }

    #[test]
    fn test_edit_input_schema_has_all_fields() {
        let tool = EditTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("oldText"));
        assert!(props.contains_key("newText"));
    }

    #[test]
    fn test_grep_input_schema_has_pattern() {
        let tool = GrepTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("pattern"));
    }

    #[test]
    fn test_ls_input_schema_optional_path() {
        let tool = LsTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }
}
