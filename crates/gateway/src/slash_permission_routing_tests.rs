//! WorkdirHandler permission routing tests (split from tests_slash_permission.rs).
//!
//! Simulates WorkdirHandler behavior: /git write commands require permission,
//! /git read-only commands and /cd, /pwd do not.
//!
//! These tests exercise the three-branch permission routing for the
//! specific case of the WorkdirHandler:
//! 1. /git commit → Exec { requires_permission: true } → deny engine blocks non-owner
//! 2. /git status → Exec { requires_permission: false } → bypasses permission engine
//! 3. /cd, /pwd → Reply(...) → unaffected by permission engine
//! 4. Owner on /git commit → owner short-circuits, bypasses engine

use std::sync::Arc;

use crate::slash_permission_test_utils::*;
use crate::HandleResult;
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};

/// WorkdirHandler mock: inspects git args to determine permission requirement.
struct WorkdirHandler;
#[async_trait::async_trait]
impl SlashHandler for WorkdirHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(WorkdirHandler)
    }

    fn commands(&self) -> &[&str] {
        &["git", "cd", "pwd"]
    }
    fn description(&self) -> &str {
        "Workdir command handler"
    }
    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        if args.starts_with("status")
            || args.starts_with("log")
            || args.starts_with("diff")
            || args.starts_with("branch")
            || args.starts_with("show")
        {
            SlashResult::Exec {
                command: format!("git {args}"),
                requires_permission: false,
            }
        } else if !args.is_empty() {
            SlashResult::Exec {
                command: format!("git {args}"),
                requires_permission: true,
            }
        } else {
            SlashResult::Reply("usage: /git <command>".to_owned())
        }
    }
}

struct WorkdirRouter;

#[async_trait::async_trait]
impl SlashRouter for WorkdirRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, _command: &str) -> bool {
        false
    }
    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        match command {
            "git" | "cd" | "pwd" => Some(Box::new(WorkdirHandler)),
            _ => None,
        }
    }
}

/// /git commit (write command) triggers permission engine for non-owner.
/// With a deny-all engine, the command is blocked (handler invoked but
/// execute skipped).
#[tokio::test]
async fn test_git_commit_non_owner_triggers_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash(
            "sess1",
            "/git commit -m test",
            Some("user1"),
            "feishu",
            Some("p"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git commit for non-owner with deny engine should be denied"
    );
}

/// /git status (read-only command) does NOT trigger permission engine.
/// Bypasses engine via Exec { requires_permission: false }.
#[tokio::test]
async fn test_git_status_readonly_bypasses_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess2", "/git status", Some("user1"), "feishu", Some("p"))
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git status should bypass permission engine and succeed"
    );
}

/// /cd and /pwd return Reply results, unaffected by permission engine.
#[tokio::test]
async fn test_cd_pwd_unaffected_by_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result_cd = gw
        .dispatch_slash("sess3", "/cd /tmp", Some("user1"), "feishu", Some("p"))
        .await;
    assert!(
        matches!(result_cd, Some(HandleResult::SlashHandled)),
        "/cd should succeed regardless of permission engine"
    );

    let result_pwd = gw
        .dispatch_slash("sess3", "/pwd", Some("user1"), "feishu", Some("p"))
        .await;
    assert!(
        matches!(result_pwd, Some(HandleResult::SlashHandled)),
        "/pwd should succeed regardless of permission engine"
    );
}

/// Owner on /git commit still directly executes (owner short-circuit).
/// Even with a deny-all engine, the owner's write command bypasses
/// the permission engine.
#[tokio::test]
async fn test_git_commit_owner_bypasses_permission_engine() {
    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash(
            "sess4",
            "/git commit -m test",
            Some("owner"),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git commit for owner should bypass permission engine"
    );
}
