//! Unit tests for [`WorkflowHandler`](crate::handler::WorkflowHandler)
//! parse paths and the `/workflow` registration contract.

use super::*;

fn make_slash_context() -> SlashContext {
    SlashContext {
        command: "workflow".to_string(),
        sender_id: "owner".to_string(),
        session_id: "s1".to_string(),
        channel: "test".to_string(),
    }
}

// ── Parse paths（解析路径） ──────────────────────────────────────────

#[tokio::test]
async fn test_empty_args_return_usage_hint() {
    let handler = WorkflowHandler::new();
    let result = handler.handle("", &make_slash_context()).await;
    match result {
        SlashResult::Reply(msg) => assert!(msg.contains("用法"), "usage hint: {msg}"),
        other => panic!("expected Reply usage hint, got {other:?}"),
    }
}

#[tokio::test]
async fn test_whitespace_only_args_return_usage_hint() {
    let handler = WorkflowHandler::new();
    let result = handler.handle("   ", &make_slash_context()).await;
    match result {
        SlashResult::Reply(msg) => assert!(msg.contains("用法"), "usage hint: {msg}"),
        other => panic!("expected Reply usage hint, got {other:?}"),
    }
}

#[tokio::test]
async fn test_name_returns_workflow_start() {
    let handler = WorkflowHandler::new();
    let result = handler.handle("Test WF", &make_slash_context()).await;
    match result {
        SlashResult::WorkflowStart { name } => assert_eq!(name, "Test WF"),
        other => panic!("expected WorkflowStart, got {other:?}"),
    }
}

#[tokio::test]
async fn test_name_is_trimmed() {
    let handler = WorkflowHandler::new();
    let result = handler.handle("  Deploy  ", &make_slash_context()).await;
    match result {
        SlashResult::WorkflowStart { name } => assert_eq!(name, "Deploy"),
        other => panic!("expected WorkflowStart, got {other:?}"),
    }
}

// ── Handler contract（注册契约） ─────────────────────────────────────

#[test]
fn test_contract_commands_immediate_and_permission() {
    let handler = WorkflowHandler::new();
    assert_eq!(handler.commands(), &["workflow"]);
    assert!(!handler.immediate("workflow", ""), "must be non-immediate");
    assert!(
        !handler.requires_permission(),
        "owner check lives in the gateway, not the handler"
    );
    assert_eq!(handler.description(), "启动受控工作流");
}

#[tokio::test]
async fn test_clone_box_preserves_parsing() {
    let handler = WorkflowHandler::new();
    let cloned = handler.clone_box();
    match cloned.handle("Test WF", &make_slash_context()).await {
        SlashResult::WorkflowStart { name } => assert_eq!(name, "Test WF"),
        other => panic!("expected WorkflowStart from cloned handler, got {other:?}"),
    }
}
