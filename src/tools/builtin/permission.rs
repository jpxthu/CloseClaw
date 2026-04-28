//! Built-in meta tool — PermissionQuery.
//!
//! Allows the LLM to query which tools the current agent is permitted to use.

use crate::tools::{Tool, ToolContext, ToolFlags};

use serde_json::Value;

// ---------------------------------------------------------------------------
// PermissionQueryTool
// ---------------------------------------------------------------------------

/// Queries the tool permission profile for the current agent.
///
/// # What it does
/// Returns a JSON object describing which tool groups or individual tools
/// the calling agent (`ctx.agent_id`) is allowed to invoke.
///
/// # Permission model
/// Permissions are checked at call time; this tool only describes the
/// agent's own permissions. The actual permission enforcement happens in
/// the tool-calling pipeline.
///
/// This tool is always loaded in the index (`is_deferred_by_default = false`).
pub struct PermissionQueryTool;

impl PermissionQueryTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for PermissionQueryTool {
    fn name(&self) -> &str {
        "PermissionQuery"
    }

    fn group(&self) -> &str {
        "meta"
    }

    fn summary(&self) -> String {
        "Query current agent tool permissions".to_string()
    }

    fn detail(&self) -> String {
        "Query the tool permission profile for the current agent.\
         Returns a JSON object describing which tool groups or individual \
         tools the calling agent is permitted to invoke.\
         \n\nThe returned object has the form:\
         `{allowed_groups: [\"file_ops\", ...], denied_tools: [...]}`.\
         \n\nIf `allowed_groups` is empty, the agent has no general tool access \
         and must rely on explicit grants in `denied_tools`.\
         \n\nUse this tool at the start of a session to understand what the \
         agent can and cannot do."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: true,
            is_read_only: true,
            is_destructive: false,
            is_expensive: false,
            is_deferred_by_default: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_query_name_group() {
        let tool = PermissionQueryTool::new();
        assert_eq!(tool.name(), "PermissionQuery");
        assert_eq!(tool.group(), "meta");
    }

    #[test]
    fn test_permission_query_summary_len() {
        let tool = PermissionQueryTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_permission_query_flags() {
        let tool = PermissionQueryTool::new();
        let flags = tool.flags();
        assert!(!flags.is_deferred_by_default);
        assert!(flags.is_read_only);
        assert!(flags.is_concurrency_safe);
    }

    #[test]
    fn test_permission_query_input_schema_no_required() {
        let tool = PermissionQueryTool::new();
        let schema = tool.input_schema();
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn test_permission_query_detail_contains_profile() {
        let tool = PermissionQueryTool::new();
        let detail = tool.detail();
        assert!(detail.contains("allowed_groups") || detail.contains("permission"));
    }
}
