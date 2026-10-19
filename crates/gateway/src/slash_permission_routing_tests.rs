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
//!
//! Also hosts the Step 1.5 Owner-criterion dimension tests (checkpoint
//! `get_sender_id` 口径): ① no bound Owner → nobody short-circuits;
//! ② sender ≠ session Owner → engine decides; ③ terminal-uid owner
//! short-circuits even under a deny-all engine.

use std::sync::{Arc, Mutex};

use crate::gateway_test_utils::SpyPlugin;
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

/// The session Owner (checkpoint sender id) on /git commit still directly
/// executes (owner short-circuit). Even with a deny-all engine, the
/// owner's write command bypasses the permission engine.
#[tokio::test]
async fn test_git_commit_owner_bypasses_permission_engine() {
    let owner = "ou_session_owner";
    let gw = make_gateway_with_owner("sess4", Some(owner)).await;
    gw.set_slash_dispatcher(Arc::new(WorkdirRouter)).await;
    gw.set_permission_engine(deny_engine()).await;
    let result = gw
        .dispatch_slash(
            "sess4",
            "/git commit -m test",
            Some(owner),
            "feishu",
            Some("p"),
        )
        .await;
    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "/git commit for owner should bypass permission engine"
    );
}

// ===========================================================================
// Step 1.5 — Owner 判据口径统一行为维度（checkpoint get_sender_id 口径）
// ===========================================================================
// 引擎侧维度见 permission crate `engine_owner_tests.rs`，审批侧维度见
// `approval_owner_criterion_tests.rs`；④（deny-all 引擎下会话 Owner
// 直通）由上方 `test_git_commit_owner_bypasses_permission_engine` 与
// `tests_slash_permission::test_owner_slash_direct_dispatch` 覆盖。

/// Permission-gated command handler returning a plain Reply (no `Exec`
/// side effects — `gw_exec` runs real commands) so the criterion tests
/// isolate the permission routing itself. Note `handle()` runs before
/// the permission gate in both branches — the denial discriminator is
/// the "权限不足" outbound, not handler invocation.
struct GatedReplyHandler;

#[async_trait::async_trait]
impl SlashHandler for GatedReplyHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(GatedReplyHandler)
    }

    fn commands(&self) -> &[&str] {
        &["deploy"]
    }

    fn description(&self) -> &str {
        "permission-gated reply command"
    }

    fn requires_permission(&self) -> bool {
        true
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Reply("deployed".to_owned())
    }
}

struct GatedReplyRouter;

#[async_trait::async_trait]
impl SlashRouter for GatedReplyRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        false
    }

    fn get_handler(&self, _command: &str) -> Option<Box<dyn SlashHandler>> {
        Some(Box::new(GatedReplyHandler))
    }
}

/// ① checkpoint 无 Owner（sender_id 未绑定）：字面 `"owner"` sender
/// 也不再短路——请求走引擎，deny-all 引擎拒绝（"权限不足" 出站、
/// execute 跳过）。
#[tokio::test]
async fn test_unbound_owner_session_no_shortcut_for_literal_owner_sender() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-no-owner", None).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;
    gw.set_slash_dispatcher(Arc::new(GatedReplyRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash(
            "sess-no-owner",
            "/deploy prod",
            Some("owner"),
            "mock",
            Some("p"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let msgs = spy_messages.lock().expect("spy poisoned");
    assert_eq!(
        msgs.len(),
        1,
        "no bound Owner → engine path: exactly the denial reply is sent, got: {:?}",
        *msgs
    );
    assert!(
        msgs[0].contains("权限不足"),
        "literal \"owner\" sender must be denied by the engine without a bound Owner, got: {:?}",
        msgs[0]
    );
}

/// ② sender ≠ 会话 Owner：不短路，走引擎——deny-all 引擎拒绝、
/// execute 跳过。
#[tokio::test]
async fn test_non_owner_sender_with_bound_owner_goes_through_engine() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-two", Some("ou_session_owner")).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;
    gw.set_slash_dispatcher(Arc::new(GatedReplyRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash(
            "sess-two",
            "/deploy prod",
            Some("someone_else"),
            "mock",
            Some("p"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let msgs = spy_messages.lock().expect("spy poisoned");
    assert_eq!(
        msgs.len(),
        1,
        "sender ≠ session Owner → engine decides: exactly the denial reply, got: {:?}",
        *msgs
    );
    assert!(
        msgs[0].contains("权限不足"),
        "non-owner sender must be denied by the engine, got: {:?}",
        msgs[0]
    );
}

/// ③ terminal uid 会话（sender = uid = checkpoint Owner）：真实 Owner
/// 短路放行——deny-all 引擎也不拦截（无 "权限不足" 出站）。
#[tokio::test]
async fn test_terminal_uid_owner_direct_dispatch_under_deny_engine() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-uid", Some("1000")).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;
    gw.set_slash_dispatcher(Arc::new(GatedReplyRouter)).await;
    gw.set_permission_engine(deny_engine()).await;

    let result = gw
        .dispatch_slash("sess-uid", "/deploy prod", Some("1000"), "mock", Some("p"))
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let msgs = spy_messages.lock().expect("spy poisoned");
    assert!(
        !msgs.iter().any(|m| m.contains("权限不足")),
        "uid Owner must short-circuit the deny-all engine, got: {:?}",
        *msgs
    );
}
