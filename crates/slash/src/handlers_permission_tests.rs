//! Unit tests for PermissionSlashHandler (Step 1.5).
//!
//! Covers:
//! - Normal path: each sub-command parsed to correct PermissionOperation
//! - Error path: unknown sub-command, insufficient args, empty args
//! - Boundary: single vs multiple paths, command with/without args

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers_permission::PermissionSlashHandler;
use closeclaw_common::permission_op::PermissionOperation;
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

fn assert_perm_op(result: SlashResult, expected: PermissionOperation) {
    match result {
        SlashResult::PermissionOp { op } => assert_eq!(op, expected),
        other => panic!("expected PermissionOp, got {other:?}"),
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
    assert!(h.immediate("perm"));
}

#[test]
fn test_requires_permission_returns_false() {
    let h = PermissionSlashHandler;
    assert!(!h.requires_permission());
}

// ── normal path: allow-file ─────────────────────────────────────────────────

#[tokio::test]
async fn test_allow_file_single_path() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("allow-file eda read /tmp/data/**", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddFileWhitelist {
            agent: "eda".into(),
            op: "read".into(),
            paths: vec!["/tmp/data/**".into()],
        },
    );
}

#[tokio::test]
async fn test_allow_file_multiple_paths() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle(
            "allow-file eda write /tmp/a.txt /tmp/b.txt /tmp/c.txt",
            &ctx,
        )
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddFileWhitelist {
            agent: "eda".into(),
            op: "write".into(),
            paths: vec![
                "/tmp/a.txt".into(),
                "/tmp/b.txt".into(),
                "/tmp/c.txt".into(),
            ],
        },
    );
}

// ── normal path: deny-file ──────────────────────────────────────────────────

#[tokio::test]
async fn test_deny_file_single_path() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("deny-file eda write /etc/shadow", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddFileDeny {
            agent: "eda".into(),
            op: "write".into(),
            paths: vec!["/etc/shadow".into()],
        },
    );
}

#[tokio::test]
async fn test_deny_file_multiple_paths() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("deny-file eda read /etc/** /root/**", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddFileDeny {
            agent: "eda".into(),
            op: "read".into(),
            paths: vec!["/etc/**".into(), "/root/**".into()],
        },
    );
}

// ── normal path: allow-cmd ──────────────────────────────────────────────────

#[tokio::test]
async fn test_allow_cmd_no_args() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("allow-cmd eda ls", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddCommandWhitelist {
            agent: "eda".into(),
            command: "ls".into(),
            args: vec![],
        },
    );
}

#[tokio::test]
async fn test_allow_cmd_with_args() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("allow-cmd eda git status log", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddCommandWhitelist {
            agent: "eda".into(),
            command: "git".into(),
            args: vec!["status".into(), "log".into()],
        },
    );
}

// ── normal path: deny-cmd ───────────────────────────────────────────────────

#[tokio::test]
async fn test_deny_cmd_no_args() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("deny-cmd eda rm", &ctx).await;
    assert_perm_op(
        result,
        PermissionOperation::AddCommandDeny {
            agent: "eda".into(),
            command: "rm".into(),
            args: vec![],
        },
    );
}

#[tokio::test]
async fn test_deny_cmd_with_args() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("deny-cmd eda rm -rf /", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddCommandDeny {
            agent: "eda".into(),
            command: "rm".into(),
            args: vec!["-rf".into(), "/".into()],
        },
    );
}

// ── error path: empty / unknown ─────────────────────────────────────────────

#[tokio::test]
async fn test_empty_args_returns_usage() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("", &ctx).await;
    assert_reply_contains(&result, "用法");
}

#[tokio::test]
async fn test_whitespace_only_returns_usage() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("   ", &ctx).await;
    assert_reply_contains(&result, "用法");
}

#[tokio::test]
async fn test_unknown_subcommand() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("bogus-cmd eda read /tmp", &ctx)
        .await;
    assert_reply_contains(&result, "未知子命令");
    assert_reply_contains(&result, "bogus-cmd");
}

// ── error path: insufficient args ───────────────────────────────────────────

#[tokio::test]
async fn test_allow_file_missing_agent() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("allow-file", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_allow_file_missing_op() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("allow-file eda", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_deny_file_missing_paths() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("deny-file eda read", &ctx)
        .await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_allow_cmd_missing_command() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("allow-cmd eda", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_deny_cmd_missing_agent() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("deny-cmd", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

#[tokio::test]
async fn test_allow_cmd_missing_agent() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler.handle("allow-cmd", &ctx).await;
    assert_reply_contains(&result, "参数不足");
}

// ── boundary: whitespace trimming ────────────────────────────────────────────

#[tokio::test]
async fn test_leading_trailing_whitespace_trimmed() {
    let ctx = dummy_ctx();
    let result = PermissionSlashHandler
        .handle("  allow-file eda read /tmp/f.txt  ", &ctx)
        .await;
    assert_perm_op(
        result,
        PermissionOperation::AddFileWhitelist {
            agent: "eda".into(),
            op: "read".into(),
            paths: vec!["/tmp/f.txt".into()],
        },
    );
}
