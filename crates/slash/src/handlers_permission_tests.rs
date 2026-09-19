//! Unit tests for PermissionSlashHandler.
//!
//! The four permission subcommands (allow-file, deny-file, allow-cmd, deny-cmd)
//! are intercepted by the Gateway in production and never reach this handler.
//! Tests verify:
//! - Trait metadata (real behavior)
//! - Empty / whitespace args → usage reply
//! - Unknown subcommand → error reply with usage
//! - Intercepted subcommands → `unreachable!` fail-fast (should_panic)

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_permission::PermissionSlashHandler;
use closeclaw_common::slash_router::SlashResult;

// ── helpers ──────────────────────────────────────────────────────────────────

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

fn assert_reply_contains(result: &SlashResult, needle: &str) {
    match result {
        SlashResult::Reply(text) => {
            assert!(
                text.contains(needle),
                "expected reply containing '{needle}', got: {text}"
            );
        }
        other => panic!("expected Reply containing '{needle}', got {other:?}"),
    }
}

// ── trait metadata ───────────────────────────────────────────────────────────

#[test]
fn test_commands_returns_perm() {
    let h = PermissionSlashHandler;
    assert_eq!(h.commands(), &["perm"]);
}

#[test]
fn test_description_non_empty() {
    let h = PermissionSlashHandler;
    assert!(!h.description().is_empty());
}

#[test]
fn test_immediate_returns_true() {
    let h = PermissionSlashHandler;
    assert!(h.immediate("perm", ""));
}

#[test]
fn test_requires_permission_returns_false() {
    let h = PermissionSlashHandler;
    assert!(!h.requires_permission());
}

// ── empty / whitespace args → usage ─────────────────────────────────────────

#[tokio::test]
async fn test_perm_empty_args_returns_usage() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("", &ctx).await;
    assert_reply_contains(&result, "用法");
}

#[tokio::test]
async fn test_perm_whitespace_only_returns_usage() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("   ", &ctx).await;
    assert_reply_contains(&result, "用法");
}

// ── unknown subcommand → error reply ────────────────────────────────────────

#[tokio::test]
async fn test_perm_unknown_subcommand_returns_error_reply() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("bogus-cmd eda read /tmp", &ctx)
        .await;
    assert_reply_contains(&result, "未知子命令");
    assert_reply_contains(&result, "bogus-cmd");
}

// ── intercepted subcommands → fail-fast (should_panic) ──────────────────────
//
// In production the Gateway intercepts these before the handler is reached.
// The handler's `dispatch` arms contain `unreachable!` as a deliberate
// fail-fast; calling them from tests verifies the panic contract.

#[tokio::test]
#[should_panic(expected = "intercepted by Gateway")]
async fn test_perm_allow_file_intercepted() {
    let ctx = dummy_ctx();
    PermissionSlashHandler
        .handle("allow-file eda read /tmp/data/**", &ctx)
        .await;
}

#[tokio::test]
#[should_panic(expected = "intercepted by Gateway")]
async fn test_perm_deny_file_intercepted() {
    let ctx = dummy_ctx();
    PermissionSlashHandler
        .handle("deny-file eda write /etc/shadow", &ctx)
        .await;
}

#[tokio::test]
#[should_panic(expected = "intercepted by Gateway")]
async fn test_perm_allow_cmd_intercepted() {
    let ctx = dummy_ctx();
    PermissionSlashHandler
        .handle("allow-cmd eda ls", &ctx)
        .await;
}

#[tokio::test]
#[should_panic(expected = "intercepted by Gateway")]
async fn test_perm_deny_cmd_intercepted() {
    let ctx = dummy_ctx();
    PermissionSlashHandler.handle("deny-cmd eda rm", &ctx).await;
}
