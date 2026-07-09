//! Unit tests for UserSlashHandler (Step 1.6).
//!
//! Covers:
//! - Normal path: each sub-command parsed to correct SlashResult
//! - Error path: unknown sub-command, insufficient args, invalid params
//! - Boundary: whitespace trimming, missing flags

use tempfile::TempDir;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_user::UserSlashHandler;
use closeclaw_common::permission_op::InitialPermissionSet;
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

fn handler_with_empty_config() -> (UserSlashHandler, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let handler = UserSlashHandler::new(dir.path().to_path_buf());
    (handler, dir)
}

fn handler_with_users(config_dir: &std::path::Path, users_json: &str) {
    std::fs::write(config_dir.join("users.json"), users_json).unwrap();
}

// ── trait metadata ───────────────────────────────────────────────────────────

#[test]
fn test_commands_returns_user() {
    let (h, _dir) = handler_with_empty_config();
    assert_eq!(h.commands(), &["user"]);
}

#[test]
fn test_description_non_empty() {
    let (h, _dir) = handler_with_empty_config();
    assert!(!h.description().is_empty());
}

#[test]
fn test_immediate_returns_true() {
    let (h, _dir) = handler_with_empty_config();
    assert!(h.immediate("user"));
}

#[test]
fn test_requires_permission_returns_false() {
    let (h, _dir) = handler_with_empty_config();
    assert!(!h.requires_permission());
}

// ── normal path: /user list ─────────────────────────────────────────────────

#[tokio::test]
async fn test_list_empty_no_file() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("list", &ctx).await;
    assert_reply_contains(&result, "暂无已注册用户");
}

#[tokio::test]
async fn test_list_empty_registry() {
    let (h, dir) = handler_with_empty_config();
    handler_with_users(dir.path(), r#"{"users":[]}"#);
    let ctx = dummy_ctx();
    let result = h.handle("list", &ctx).await;
    assert_reply_contains(&result, "暂无已注册用户");
}

#[tokio::test]
async fn test_list_with_users() {
    let (h, dir) = handler_with_empty_config();
    let registry = r#"{"users":[{"user_id":"ou_a","im_channel":"feishu","initial_permissions":["BasicMessaging"],"created_at":"2026-01-01T00:00:00Z"}]}"#;
    handler_with_users(dir.path(), registry);

    let ctx = dummy_ctx();
    let result = h.handle("list", &ctx).await;
    match &result {
        SlashResult::Reply(text) => {
            assert!(text.contains("ou_a"));
            assert!(text.contains("feishu"));
            assert!(text.contains("BasicMessaging"));
            assert!(text.contains("已注册用户"));
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── normal path: /user approve ──────────────────────────────────────────────

#[tokio::test]
async fn test_approve_basic_request_id() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve req-001", &ctx).await;
    match &result {
        SlashResult::UserApprove {
            request_id,
            initial_permissions,
        } => {
            assert_eq!(request_id, "req-001");
            assert_eq!(
                initial_permissions,
                &vec![InitialPermissionSet::BasicMessaging]
            );
        }
        other => panic!("expected UserApprove, got {other:?}"),
    }
}

#[tokio::test]
async fn test_approve_with_perms_basic() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve req-002 --perms basic", &ctx).await;
    match &result {
        SlashResult::UserApprove {
            request_id,
            initial_permissions,
        } => {
            assert_eq!(request_id, "req-002");
            assert_eq!(
                initial_permissions,
                &vec![InitialPermissionSet::BasicMessaging]
            );
        }
        other => panic!("expected UserApprove, got {other:?}"),
    }
}

#[tokio::test]
async fn test_approve_with_perms_basic_messaging() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h
        .handle("approve req-003 --perms basic-messaging", &ctx)
        .await;
    match &result {
        SlashResult::UserApprove {
            request_id,
            initial_permissions,
        } => {
            assert_eq!(request_id, "req-003");
            assert_eq!(
                initial_permissions,
                &vec![InitialPermissionSet::BasicMessaging]
            );
        }
        other => panic!("expected UserApprove, got {other:?}"),
    }
}

// ── normal path: /user reject ───────────────────────────────────────────────

#[tokio::test]
async fn test_reject_request_id() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("reject req-100", &ctx).await;
    match &result {
        SlashResult::UserReject { request_id } => {
            assert_eq!(request_id, "req-100");
        }
        other => panic!("expected UserReject, got {other:?}"),
    }
}

// ── error path: empty / unknown ─────────────────────────────────────────────

#[tokio::test]
async fn test_empty_args_returns_usage() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("", &ctx).await;
    assert_reply_contains(&result, "用法");
}

#[tokio::test]
async fn test_whitespace_only_returns_usage() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("   ", &ctx).await;
    assert_reply_contains(&result, "用法");
}

#[tokio::test]
async fn test_unknown_subcommand() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("bogus", &ctx).await;
    assert_reply_contains(&result, "未知子命令");
    assert_reply_contains(&result, "bogus");
}

// ── error path: insufficient args ───────────────────────────────────────────

#[tokio::test]
async fn test_approve_missing_request_id() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_reject_missing_request_id() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("reject", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_approve_perms_missing_value() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve req-001 --perms", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

// ── error path: invalid params ──────────────────────────────────────────────

#[tokio::test]
async fn test_approve_invalid_perm_set() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve req-001 --perms nonexistent", &ctx).await;
    assert_reply_contains(&result, "无效的权限集合");
    assert_reply_contains(&result, "nonexistent");
}

#[tokio::test]
async fn test_approve_unknown_flag() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("approve req-001 --bogus", &ctx).await;
    assert_reply_contains(&result, "未知参数");
    assert_reply_contains(&result, "--bogus");
}

// ── boundary: whitespace trimming ────────────────────────────────────────────

#[tokio::test]
async fn test_approve_leading_trailing_whitespace() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("  approve req-001  ", &ctx).await;
    match &result {
        SlashResult::UserApprove { request_id, .. } => {
            assert_eq!(request_id, "req-001");
        }
        other => panic!("expected UserApprove, got {other:?}"),
    }
}

#[tokio::test]
async fn test_reject_leading_trailing_whitespace() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("  reject req-200  ", &ctx).await;
    match &result {
        SlashResult::UserReject { request_id } => {
            assert_eq!(request_id, "req-200");
        }
        other => panic!("expected UserReject, got {other:?}"),
    }
}

#[tokio::test]
async fn test_list_leading_trailing_whitespace() {
    let (h, _dir) = handler_with_empty_config();
    let ctx = dummy_ctx();
    let result = h.handle("  list  ", &ctx).await;
    assert_reply_contains(&result, "暂无已注册用户");
}
