//! Shared test support for `registry.rs` test modules.
//!
//! Mounted via `#[cfg(test)]` from `lib.rs` so the registry test modules
//! (attached to `registry.rs` through `#[path]`) can share `DummyTool`
//! and `make_prompt_ctx` without depending on private items of a single
//! test file.

use crate::{PromptGenerationContext, Tool, ToolFlags};

#[derive(Default)]
pub(crate) struct DummyTool {
    pub(crate) name: String,
    pub(crate) group: String,
    pub(crate) summary_text: String,
    pub(crate) is_deferred: bool,
    pub(crate) is_read_only: bool,
    pub(crate) is_destructive: bool,
    pub(crate) is_expensive: bool,
}

impl Tool for DummyTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn group(&self) -> &str {
        &self.group
    }
    fn summary(&self) -> String {
        self.summary_text.clone()
    }
    fn detail(&self) -> String {
        format!("detail for {}", self.name)
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_deferred_by_default: self.is_deferred,
            is_read_only: self.is_read_only,
            is_destructive: self.is_destructive,
            is_expensive: self.is_expensive,
            ..Default::default()
        }
    }
}

/// Build a `PromptGenerationContext` for the named tools.
pub(crate) fn make_prompt_ctx(names: &[&str]) -> PromptGenerationContext {
    PromptGenerationContext {
        agent_id: "test-agent".to_string(),
        workdir: None,
        available_tool_names: names.iter().map(|s| s.to_string()).collect(),
        tools: None,
        disallowed_tools: None,
        session_mode: None,
        agent_role: None,
        agent_type: None,
    }
}

#[cfg(test)]
mod default_contract_tests {
    //! Pins the `DummyTool::default()` contract that 90 struct-update call
    //! sites rely on: `..Default::default()` must be equivalent to the
    //! previously explicit `false` flags and empty `summary_text`. If a field
    //! default ever drifts, those call sites would silently change semantics.

    use super::*;

    #[test]
    fn test_dummy_tool_default_field_values() {
        let tool = DummyTool::default();
        assert!(tool.name.is_empty());
        assert!(tool.group.is_empty());
        assert!(tool.summary_text.is_empty());
        assert!(!tool.is_deferred);
        assert!(!tool.is_read_only);
        assert!(!tool.is_destructive);
        assert!(!tool.is_expensive);
    }
}
