//! Built-in meta tool — ToolSearch.
//!
//! Allows the LLM to dynamically retrieve second-level tool detail
//! by keyword or exact name match.

use crate::tools::{Tool, ToolContext, ToolError, ToolFlags};

use serde_json::Value;

// ---------------------------------------------------------------------------
// ToolSearchTool
// ---------------------------------------------------------------------------

/// Dynamic tool detail lookup by keyword or exact name.
///
/// # What it does
/// When the LLM needs to understand a tool's full input schema or detail
/// description, it calls `ToolSearch` with a `query` string.
///
/// - **exact mode** (query matches a tool name verbatim): returns that tool's
///   full `detail()` + `input_schema()`.
/// - **keyword mode** (query contains non-matching text): searches all tool
///   names and summaries, returns matching tools' name + summary + group.
///
/// # Safety / Cost
/// - `is_expensive = true` (registry scan across all tools)
/// - `is_deferred_by_default = false` (always loaded in index)
pub struct ToolSearchTool;

impl ToolSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn group(&self) -> &str {
        "meta"
    }

    fn summary(&self) -> String {
        "Search for tool details by keyword or exact name".to_string()
    }

    fn detail(&self) -> String {
        "Dynamically retrieve second-level tool detail by keyword or exact name.\
         When the LLM needs the full description or input schema of a specific \
         tool, call this tool with a `query` string.\
         \n\n**Exact mode**: if `query` exactly matches a registered tool name \
         (case-insensitive), returns that tool's full `detail()` text and \
         `input_schema` JSON object.\
         \n\n**Keyword mode**: if `query` does not exactly match any tool name, \
         searches all tool names and summaries for any entry that contains \
         the query substring (case-insensitive). Returns a list of \
         `{name, group, summary}` for all matches, up to a limit of 10.\
         \n\nThis is the only way to load a tool's second-level detail into the \
         context, since first-level index only shows group + tool name list."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query: exact tool name or keyword substring"
                }
            },
            "required": ["query"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_read_only: true,
            is_destructive: false,
            is_expensive: true,
            is_deferred_by_default: false,
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
    fn test_toolsearch_name_group() {
        let tool = ToolSearchTool::new();
        assert_eq!(tool.name(), "ToolSearch");
        assert_eq!(tool.group(), "meta");
    }

    #[test]
    fn test_toolsearch_summary_len() {
        let tool = ToolSearchTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_toolsearch_flags() {
        let tool = ToolSearchTool::new();
        let flags = tool.flags();
        assert!(!flags.is_deferred_by_default);
        assert!(flags.is_expensive);
        assert!(flags.is_read_only);
    }

    #[test]
    fn test_toolsearch_input_schema_has_query() {
        let tool = ToolSearchTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("query"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    #[test]
    fn test_toolsearch_detail_contains_modes() {
        let tool = ToolSearchTool::new();
        let detail = tool.detail();
        assert!(detail.contains("Exact mode"));
        assert!(detail.contains("Keyword mode"));
    }
}
