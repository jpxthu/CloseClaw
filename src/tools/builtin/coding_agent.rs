//! Built-in tools — coding agent (stub Tool implementation).
//!
//! This is a placeholder stub. Full implementation tracked in issue #282.

use crate::tools::{Tool, ToolContext, ToolFlags};

use serde_json::Value;

// ---------------------------------------------------------------------------
// CodingAgentTool
// ---------------------------------------------------------------------------

/// Stub tool for delegating coding tasks to a coding agent (Codex, Claude Code, etc.).
///
/// Real implementation tracked in issue #282.
pub struct CodingAgentTool;

impl Default for CodingAgentTool {
    fn default() -> Self {
        Self
    }
}

impl CodingAgentTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for CodingAgentTool {
    fn name(&self) -> &str {
        "CodingAgent"
    }

    fn group(&self) -> &str {
        "coding_agent"
    }

    fn summary(&self) -> String {
        "Delegate coding tasks to a coding agent".to_string()
    }

    fn detail(&self) -> String {
        "Stub — real implementation tracked in issue #282. \
         This tool delegates coding tasks (building features, reviewing PRs, \
         refactoring, iterative coding) to an external coding agent such as Codex, \
         Claude Code, or Pi. Returns a session handle for tracking progress."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Natural language description of the coding task"
                },
                "runtime": {
                    "type": "string",
                    "description": "Coding agent runtime: 'codex', 'claude_code', or 'pi'",
                    "enum": ["codex", "claude_code", "pi"]
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for the coding agent"
                }
            },
            "required": ["task", "runtime"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_concurrency_safe: false,
            is_expensive: true,
            is_deferred_by_default: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn test_coding_agent_name() {
        let tool = CodingAgentTool::new();
        assert_eq!(tool.name(), "CodingAgent");
    }

    #[test]
    fn test_coding_agent_group() {
        let tool = CodingAgentTool::new();
        assert_eq!(tool.group(), "coding_agent");
    }

    #[test]
    fn test_coding_agent_summary_len() {
        let tool = CodingAgentTool::new();
        assert!(tool.summary().len() <= 50);
    }

    #[test]
    fn test_coding_agent_flags_deferred() {
        let tool = CodingAgentTool::new();
        assert!(tool.flags().is_deferred_by_default);
        assert!(!tool.flags().is_read_only);
    }

    #[test]
    fn test_coding_agent_flags_expensive() {
        let tool = CodingAgentTool::new();
        assert!(tool.flags().is_expensive);
    }

    #[test]
    fn test_coding_agent_schema_has_task() {
        let tool = CodingAgentTool::new();
        let schema = tool.input_schema();
        let props = schema.pointer("/properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("task"));
        assert!(props.contains_key("runtime"));
        let required = schema.pointer("/required").unwrap().as_array().unwrap();
        assert!(required.contains(&serde_json::json!("task")));
        assert!(required.contains(&serde_json::json!("runtime")));
    }

    #[test]
    fn test_coding_agent_detail_mentions_282() {
        let tool = CodingAgentTool::new();
        assert!(tool.detail().contains("#282"));
    }
}
