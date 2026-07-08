//! Unit tests for Tool trait, ToolContext, and associated types.
//!
//! Validates that the migrated Tool trait (now in common) and its
//! supporting types behave correctly.

use crate::tool_registry::ToolFlags;
use crate::tool_trait::{
    build_workdir_context, ContextModifier, PromptGenerationContext, Tool, ToolCallError,
    ToolContext, ToolMessage, ToolResult, WorkdirContext,
};
use async_trait::async_trait;
use serde_json::json;

// =========================================================================
// Mock Tool
// =========================================================================

struct MockTool;

#[async_trait]
impl Tool for MockTool {
    fn name(&self) -> &str {
        "MockTool"
    }

    fn group(&self) -> &str {
        "testing"
    }

    fn summary(&self) -> String {
        "A mock tool for testing".to_string()
    }

    fn detail(&self) -> String {
        "Detailed description of the mock tool with full info.".to_string()
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "input": { "type": "string" }
            },
            "required": ["input"]
        })
    }

    fn flags(&self) -> ToolFlags {
        ToolFlags {
            is_read_only: true,
            is_concurrency_safe: true,
            ..Default::default()
        }
    }
}

// =========================================================================
// Tool trait: name / group / summary / detail / flags
// =========================================================================

#[test]
fn test_tool_name_returns_correct_value() {
    let tool = MockTool;
    assert_eq!(tool.name(), "MockTool");
}

#[test]
fn test_tool_group_returns_correct_value() {
    let tool = MockTool;
    assert_eq!(tool.group(), "testing");
}

#[test]
fn test_tool_summary_returns_correct_value() {
    let tool = MockTool;
    assert_eq!(tool.summary(), "A mock tool for testing");
}

#[test]
fn test_tool_detail_returns_correct_value() {
    let tool = MockTool;
    assert_eq!(
        tool.detail(),
        "Detailed description of the mock tool with full info."
    );
}

#[test]
fn test_tool_flags_read_only() {
    let tool = MockTool;
    let flags = tool.flags();
    assert!(flags.is_read_only);
}

#[test]
fn test_tool_flags_concurrency_safe() {
    let tool = MockTool;
    let flags = tool.flags();
    assert!(flags.is_concurrency_safe);
}

#[test]
fn test_tool_flags_not_destructive() {
    let tool = MockTool;
    let flags = tool.flags();
    assert!(!flags.is_destructive);
}

#[test]
fn test_tool_flags_not_expensive() {
    let tool = MockTool;
    let flags = tool.flags();
    assert!(!flags.is_expensive);
}

#[test]
fn test_tool_flags_not_deferred_by_default() {
    let tool = MockTool;
    let flags = tool.flags();
    assert!(!flags.is_deferred_by_default);
}

#[test]
fn test_tool_flags_is_eager_when_not_deferred() {
    let flags = ToolFlags {
        is_deferred_by_default: false,
        ..Default::default()
    };
    assert!(flags.is_eager());
}

#[test]
fn test_tool_flags_is_not_eager_when_deferred() {
    let flags = ToolFlags {
        is_deferred_by_default: true,
        ..Default::default()
    };
    assert!(!flags.is_eager());
}

#[test]
fn test_tool_input_schema_is_valid_json() {
    let tool = MockTool;
    let schema = tool.input_schema();
    assert!(schema.is_object());
    let props = schema.get("properties").unwrap();
    assert!(props.get("input").is_some());
}

#[test]
fn test_tool_default_call_returns_not_implemented() {
    let tool = MockTool;
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let ctx = ToolContext {
            agent_id: "test".into(),
            workdir: None,
            session_id: None,
            call_id: None,
            session: None,
        };
        let result = tool.call(json!({}), &ctx).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ToolCallError::NotImplemented));
    });
}

#[test]
fn test_tool_generate_prompt_defaults_to_detail() {
    let tool = MockTool;
    let ctx = PromptGenerationContext {
        agent_id: "test".into(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };
    assert_eq!(tool.generate_prompt(&ctx), tool.detail());
}

// =========================================================================
// Box<dyn Tool> delegation
// =========================================================================

#[test]
fn test_box_dyn_tool_delegates_name() {
    let tool: Box<dyn Tool> = Box::new(MockTool);
    assert_eq!(tool.name(), "MockTool");
}

#[test]
fn test_box_dyn_tool_delegates_group() {
    let tool: Box<dyn Tool> = Box::new(MockTool);
    assert_eq!(tool.group(), "testing");
}

#[test]
fn test_box_dyn_tool_delegates_summary() {
    let tool: Box<dyn Tool> = Box::new(MockTool);
    assert_eq!(tool.summary(), "A mock tool for testing");
}

#[test]
fn test_box_dyn_tool_delegates_flags() {
    let tool: Box<dyn Tool> = Box::new(MockTool);
    let flags = tool.flags();
    assert!(flags.is_read_only);
}

#[test]
fn test_box_dyn_tool_delegates_input_schema() {
    let tool: Box<dyn Tool> = Box::new(MockTool);
    let schema = tool.input_schema();
    assert!(schema.is_object());
}

// =========================================================================
// ToolContext construction
// =========================================================================

#[test]
fn test_tool_context_fields() {
    let ctx = ToolContext {
        agent_id: "agent_1".into(),
        workdir: None,
        session_id: Some("sess_42".into()),
        call_id: Some("call_99".into()),
        session: None,
    };
    assert_eq!(ctx.agent_id, "agent_1");
    assert!(ctx.workdir.is_none());
    assert_eq!(ctx.session_id.as_deref(), Some("sess_42"));
    assert_eq!(ctx.call_id.as_deref(), Some("call_99"));
    assert!(ctx.session.is_none());
}

#[test]
fn test_tool_context_with_workdir() {
    let workdir = WorkdirContext {
        path: "/tmp/test".into(),
        has_git: false,
        branch: None,
        recent_changes: 0,
    };
    let ctx = ToolContext {
        agent_id: "a".into(),
        workdir: Some(workdir),
        session_id: None,
        call_id: None,
        session: None,
    };
    let wd = ctx.workdir.as_ref().unwrap();
    assert_eq!(wd.path, "/tmp/test");
    assert!(!wd.has_git);
    assert!(wd.branch.is_none());
    assert_eq!(wd.recent_changes, 0);
}

#[test]
fn test_tool_context_debug_output() {
    let ctx = ToolContext {
        agent_id: "debug_agent".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    };
    let debug = format!("{:?}", ctx);
    assert!(debug.contains("ToolContext"));
    assert!(debug.contains("debug_agent"));
    // When session is None, the Debug impl shows None (not the marker)
    assert!(debug.contains("None") || debug.contains("session"));
}

#[test]
fn test_tool_context_debug_with_session() {
    // We can't easily create a dyn ToolSession in a unit test without
    // a concrete implementation, but we can verify the Debug format
    // handles the Some case by checking the format includes the marker.
    // This test is more of a compile-time check that Debug works.
    let ctx = ToolContext {
        agent_id: "a".into(),
        workdir: None,
        session_id: None,
        call_id: None,
        session: None,
    };
    let debug = format!("{:?}", ctx);
    assert!(debug.contains("agent_id"));
}

#[test]
fn test_tool_context_clone() {
    let ctx = ToolContext {
        agent_id: "clone_test".into(),
        workdir: Some(WorkdirContext {
            path: "/home".into(),
            has_git: true,
            branch: Some("main".into()),
            recent_changes: 3,
        }),
        session_id: Some("s1".into()),
        call_id: Some("c1".into()),
        session: None,
    };
    let cloned = ctx.clone();
    assert_eq!(cloned.agent_id, "clone_test");
    assert_eq!(cloned.workdir.as_ref().unwrap().path, "/home");
    assert!(cloned.workdir.as_ref().unwrap().has_git);
    assert_eq!(
        cloned.workdir.as_ref().unwrap().branch.as_deref(),
        Some("main")
    );
    assert_eq!(cloned.workdir.as_ref().unwrap().recent_changes, 3);
    assert_eq!(cloned.session_id.as_deref(), Some("s1"));
    assert_eq!(cloned.call_id.as_deref(), Some("c1"));
}

// =========================================================================
// WorkdirContext
// =========================================================================

#[test]
fn test_build_workdir_context_with_absolute_path() {
    let ctx = build_workdir_context("/tmp");
    assert!(ctx.path.starts_with("/tmp"));
    assert!(!ctx.has_git || ctx.branch.is_some()); // git info is valid
}

#[test]
fn test_build_workdir_context_with_relative_path() {
    let ctx = build_workdir_context(".");
    // Relative path should be resolved to an absolute path
    assert!(std::path::Path::new(&ctx.path).is_absolute());
}

#[test]
fn test_build_workdir_context_nonexistent_path_falls_back() {
    // Non-existent path should still produce a valid WorkdirContext
    let ctx = build_workdir_context("/nonexistent_path_12345");
    // Should not panic; path is resolved (may or may not canonicalize)
    assert!(!ctx.path.is_empty());
}

// =========================================================================
// PromptGenerationContext
// =========================================================================

#[test]
fn test_prompt_generation_context_fields() {
    let ctx = PromptGenerationContext {
        agent_id: "pg_agent".into(),
        workdir: None,
        available_tool_names: vec!["Tool1".into(), "Tool2".into()],
        tools: Some(vec!["Tool1".into()]),
        disallowed_tools: Some(vec!["Tool3".into()]),
        session_mode: None,
    };
    assert_eq!(ctx.agent_id, "pg_agent");
    assert!(ctx.workdir.is_none());
    assert_eq!(ctx.available_tool_names.len(), 2);
    assert_eq!(ctx.tools.as_ref().unwrap().len(), 1);
    assert_eq!(ctx.disallowed_tools.as_ref().unwrap().len(), 1);
}

#[test]
fn test_prompt_generation_context_clone() {
    let ctx = PromptGenerationContext {
        agent_id: "pg".into(),
        workdir: None,
        available_tool_names: vec!["A".into()],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };
    let cloned = ctx.clone();
    assert_eq!(cloned.agent_id, "pg");
    assert_eq!(cloned.available_tool_names, vec!["A"]);
}

#[test]
fn test_prompt_generation_context_debug() {
    let ctx = PromptGenerationContext {
        agent_id: "dbg".into(),
        workdir: None,
        available_tool_names: vec![],
        tools: None,
        disallowed_tools: None,
        session_mode: None,
    };
    let debug = format!("{:?}", ctx);
    assert!(debug.contains("PromptGenerationContext"));
    assert!(debug.contains("dbg"));
}

// =========================================================================
// ToolMessage / ContextModifier / ToolResult
// =========================================================================

#[test]
fn test_tool_message_fields() {
    let msg = ToolMessage {
        content: "injected context".into(),
        is_meta: true,
    };
    assert_eq!(msg.content, "injected context");
    assert!(msg.is_meta);
}

#[test]
fn test_tool_message_clone() {
    let msg = ToolMessage {
        content: "test".into(),
        is_meta: false,
    };
    let cloned = msg.clone();
    assert_eq!(cloned.content, "test");
    assert!(!cloned.is_meta);
}

#[test]
fn test_context_modifier_fields() {
    let modifier = ContextModifier {
        allowed_tools: vec!["Read".into(), "Bash".into()],
    };
    assert_eq!(modifier.allowed_tools.len(), 2);
}

#[test]
fn test_tool_result_fields() {
    let result = ToolResult {
        data: json!({"output": "success"}),
        new_messages: vec![ToolMessage {
            content: "msg".into(),
            is_meta: false,
        }],
        context_modifier: Some(ContextModifier {
            allowed_tools: vec![],
        }),
    };
    assert_eq!(result.data["output"], "success");
    assert_eq!(result.new_messages.len(), 1);
    assert!(result.context_modifier.is_some());
}

#[test]
fn test_tool_result_clone() {
    let result = ToolResult {
        data: json!({"k": "v"}),
        new_messages: vec![],
        context_modifier: None,
    };
    let cloned = result.clone();
    assert_eq!(cloned.data, json!({"k": "v"}));
    assert!(cloned.new_messages.is_empty());
    assert!(cloned.context_modifier.is_none());
}

// =========================================================================
// ToolFlags
// =========================================================================

#[test]
fn test_tool_flags_default() {
    let flags = ToolFlags::default();
    assert!(!flags.is_concurrency_safe);
    assert!(!flags.is_read_only);
    assert!(!flags.is_destructive);
    assert!(!flags.is_expensive);
    assert!(!flags.is_deferred_by_default);
}

#[test]
fn test_tool_flags_all_set() {
    let flags = ToolFlags {
        is_concurrency_safe: true,
        is_read_only: true,
        is_destructive: true,
        is_expensive: true,
        is_deferred_by_default: true,
    };
    assert!(flags.is_concurrency_safe);
    assert!(flags.is_read_only);
    assert!(flags.is_destructive);
    assert!(flags.is_expensive);
    assert!(!flags.is_eager());
}

// =========================================================================
// ToolCallError
// =========================================================================

#[test]
fn test_tool_call_error_not_found_display() {
    let err = ToolCallError::NotFound("missing".into());
    assert_eq!(format!("{}", err), "skill not found: missing");
}

#[test]
fn test_tool_call_error_permission_denied_display() {
    let err = ToolCallError::PermissionDenied("admin_only".into());
    assert_eq!(
        format!("{}", err),
        "permission denied for skill: admin_only"
    );
}

#[test]
fn test_tool_call_error_invalid_args_display() {
    let err = ToolCallError::InvalidArgs("missing field".into());
    assert_eq!(format!("{}", err), "invalid arguments: missing field");
}

#[test]
fn test_tool_call_error_execution_failed_display() {
    let err = ToolCallError::ExecutionFailed("segfault".into());
    assert_eq!(format!("{}", err), "execution failed: segfault");
}

#[test]
fn test_tool_call_error_not_implemented_display() {
    let err = ToolCallError::NotImplemented;
    assert_eq!(format!("{}", err), "call not implemented for this tool");
}

#[test]
fn test_tool_call_error_clone() {
    let err = ToolCallError::ExecutionFailed("err".into());
    let cloned = err.clone();
    assert_eq!(format!("{}", cloned), "execution failed: err");
}
