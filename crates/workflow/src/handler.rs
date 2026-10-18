//! `/workflow` slash entry: parse-only [`WorkflowHandler`].
//!
//! Per `docs/design/workflow/workflow-tools.md`（触发机制）, the workflow
//! module provides the [`WorkflowHandler`] as the `SlashHandler`
//! implementation for the `/workflow` route. It only parses the workflow
//! `name` and returns
//! [`SlashResult::WorkflowStart`](closeclaw_common::slash_router::SlashResult::WorkflowStart)
//! — no side effects happen here. The Gateway executes the launch through
//! `WorkflowControl::start` after enforcing the Owner check, then replies
//! with the confirmation or error text.

use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult};

/// `/workflow <name>` — parse the workflow name, produce no side effects.
///
/// - empty / whitespace-only args → usage hint [`SlashResult::Reply`]
/// - otherwise → `SlashResult::WorkflowStart { name }` with the trimmed name
///
/// Contract: `commands() == ["workflow"]`, non-immediate, no permission
/// marker — the Owner check is enforced by the Gateway before dispatch.
#[derive(Clone, Default)]
pub struct WorkflowHandler;

impl WorkflowHandler {
    /// Create the stateless workflow slash handler.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl SlashHandler for WorkflowHandler {
    fn commands(&self) -> &[&str] {
        &["workflow"]
    }

    fn description(&self) -> &str {
        "启动受控工作流"
    }

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        false
    }

    fn requires_permission(&self) -> bool {
        false
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        let name = args.trim();
        if name.is_empty() {
            return SlashResult::Reply("用法：/workflow <name>".to_owned());
        }
        SlashResult::WorkflowStart {
            name: name.to_owned(),
        }
    }

    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
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
    async fn empty_args_return_usage_hint() {
        let handler = WorkflowHandler::new();
        let result = handler.handle("", &make_slash_context()).await;
        match result {
            SlashResult::Reply(msg) => assert!(msg.contains("用法"), "usage hint: {msg}"),
            other => panic!("expected Reply usage hint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn whitespace_only_args_return_usage_hint() {
        let handler = WorkflowHandler::new();
        let result = handler.handle("   ", &make_slash_context()).await;
        match result {
            SlashResult::Reply(msg) => assert!(msg.contains("用法"), "usage hint: {msg}"),
            other => panic!("expected Reply usage hint, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn name_returns_workflow_start() {
        let handler = WorkflowHandler::new();
        let result = handler.handle("Test WF", &make_slash_context()).await;
        match result {
            SlashResult::WorkflowStart { name } => assert_eq!(name, "Test WF"),
            other => panic!("expected WorkflowStart, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn name_is_trimmed() {
        let handler = WorkflowHandler::new();
        let result = handler.handle("  Deploy  ", &make_slash_context()).await;
        match result {
            SlashResult::WorkflowStart { name } => assert_eq!(name, "Deploy"),
            other => panic!("expected WorkflowStart, got {other:?}"),
        }
    }

    // ── Handler contract（注册契约） ─────────────────────────────────────

    #[test]
    fn contract_commands_immediate_and_permission() {
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
    async fn clone_box_preserves_parsing() {
        let handler = WorkflowHandler::new();
        let cloned = handler.clone_box();
        match cloned.handle("Test WF", &make_slash_context()).await {
            SlashResult::WorkflowStart { name } => assert_eq!(name, "Test WF"),
            other => panic!("expected WorkflowStart from cloned handler, got {other:?}"),
        }
    }
}
