//! Tests for SlashDispatcher routing (post Step 1.2).

use std::sync::Arc;

use crate::slash_permission_test_utils::*;
use crate::HandleResult;
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};

struct SimpleHandler {
    command: &'static str,
    requires_permission: bool,
}

#[async_trait::async_trait]
impl SlashHandler for SimpleHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(SimpleHandler {
            command: self.command,
            requires_permission: self.requires_permission,
        })
    }

    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }
    fn description(&self) -> &str {
        "Simple test handler"
    }
    fn requires_permission(&self) -> bool {
        self.requires_permission
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("ok".into())
    }
}

struct DefaultTestRouter;

#[async_trait::async_trait]
impl SlashRouter for DefaultTestRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        match command {
            "help" => Some(Box::new(SimpleHandler {
                command: "help",
                requires_permission: false,
            })),
            "exec" => Some(Box::new(SimpleHandler {
                command: "exec",
                requires_permission: true,
            })),
            _ => None,
        }
    }
}

/// `/perm` enters SlashDispatcher (no longer intercepted at Gateway level).
#[tokio::test]
async fn test_perm_cmd_enters_slash_dispatcher() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(DefaultTestRouter)).await;
    let result = gw
        .dispatch_slash(
            "sess_perm",
            "/perm allow-cmd git commit 允许提交代码",
            Some("owner"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

/// `/user approve` enters SlashDispatcher (no longer intercepted at Gateway level).
#[tokio::test]
async fn test_user_approve_enters_slash_dispatcher() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(DefaultTestRouter)).await;
    let result = gw
        .dispatch_slash(
            "sess_user",
            "/user approve req-123",
            Some("owner"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}

/// `/user reject` enters SlashDispatcher (no longer intercepted at Gateway level).
#[tokio::test]
async fn test_user_reject_enters_slash_dispatcher() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(DefaultTestRouter)).await;
    let result = gw
        .dispatch_slash(
            "sess_user",
            "/user reject req-456",
            Some("owner"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(matches!(result, Some(HandleResult::SlashHandled)));
}
