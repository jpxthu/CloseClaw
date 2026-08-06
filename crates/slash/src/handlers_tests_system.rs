//! Tests for `/system list` and `/system` (no args) branches.

use std::sync::Arc;

use crate::context::SlashContext;
use crate::handler::SlashHandler;
use crate::handlers::SystemHandler;
use closeclaw_common::slash_router::{SlashResult, SystemAppendAction};
use closeclaw_gateway::session_manager::SessionManager;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_sm() -> Arc<SessionManager> {
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = closeclaw_gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None,
        None,
        ReasoningLevel::default(),
    ))
}

async fn create_test_session(sm: &SessionManager) -> String {
    use closeclaw_gateway::Message;

    let msg = Message {
        id: "sys-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    sm.find_or_create("feishu", &msg, None)
        .await
        .expect("session")
}

fn make_ctx(session_id: &str) -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: session_id.to_owned(),
        channel: "test_channel".to_owned(),
    }
}

/// Add system append content directly to a session via its ConversationSession.
async fn seed_system_append(sm: &SessionManager, session_id: &str, content: &str) {
    let conv = sm
        .get_conversation_session(session_id)
        .await
        .expect("session active");
    let mut cs = conv.write().await;
    cs.add_system_append(content.to_owned());
}

fn dummy_ctx() -> SlashContext {
    SlashContext {
        command: String::new(),
        sender_id: "test_sender".to_owned(),
        session_id: "test_session".to_owned(),
        channel: "test_channel".to_owned(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_system_list_with_content() {
    let sm = make_sm();
    let sid = create_test_session(&sm).await;
    seed_system_append(&sm, &sid, "请始终使用中文回复").await;
    seed_system_append(&sm, &sid, "不要使用 markdown").await;

    let h = SystemHandler::new(Arc::clone(&sm));
    let ctx = make_ctx(&sid);
    match h.handle("list", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("[0]"), "should contain index 0, got: {text}");
            assert!(text.contains("请始终使用中文回复"), "got: {text}");
            assert!(text.contains("[1]"), "should contain index 1, got: {text}");
            assert!(text.contains("不要使用 markdown"), "got: {text}");
        }
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_system_list_empty() {
    let sm = make_sm();
    let sid = create_test_session(&sm).await;

    let h = SystemHandler::new(Arc::clone(&sm));
    let ctx = make_ctx(&sid);
    match h.handle("list", &ctx).await {
        SlashResult::Reply(text) => {
            assert_eq!(text, "无追加指令", "got: {text}");
        }
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_system_no_args_empty() {
    // /system (no args) with empty appends should also return "无追加指令"
    let sm = make_sm();
    let sid = create_test_session(&sm).await;

    let h = SystemHandler::new(Arc::clone(&sm));
    let ctx = make_ctx(&sid);
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert_eq!(
                text, "无追加指令",
                "/system no-args empty list should match design doc: {text}"
            );
        }
        _other => panic!("expected Reply"),
    }
}

#[tokio::test]
async fn test_system_no_args_with_content() {
    let sm = make_sm();
    let sid = create_test_session(&sm).await;
    seed_system_append(&sm, &sid, "第一条指令").await;
    seed_system_append(&sm, &sid, "第二条指令").await;
    seed_system_append(&sm, &sid, "第三条指令").await;

    let h = SystemHandler::new(Arc::clone(&sm));
    let ctx = make_ctx(&sid);
    match h.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(text.contains("[0] 第一条指令"), "got: {text}");
            assert!(text.contains("[1] 第二条指令"), "got: {text}");
            assert!(text.contains("[2] 第三条指令"), "got: {text}");
        }
        _other => panic!("expected Reply"),
    }
}

// ── Step 1.4: /system add 500-char length limit ───────────────────────────

/// Verify `/system add` rejects content exceeding 500 characters.
#[tokio::test]
async fn test_system_add_exceeds_500_chars_rejected() {
    let sm = make_sm();
    let h = SystemHandler::new(Arc::clone(&sm));
    let content = "a".repeat(501);
    let args = format!("add {content}");
    match h.handle(&args, &dummy_ctx()).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("超过 500 字符限制"),
                "should mention 500 char limit, got: {text}"
            );
        }
        other => panic!("expected Reply for >500 chars, got {other:?}"),
    }
}

/// Verify `/system add` accepts content of exactly 500 characters.
#[tokio::test]
async fn test_system_add_exactly_500_chars_accepted() {
    let sm = make_sm();
    let h = SystemHandler::new(Arc::clone(&sm));
    let content = "b".repeat(500);
    let args = format!("add {content}");
    match h.handle(&args, &dummy_ctx()).await {
        SlashResult::SystemAppend {
            action: SystemAppendAction::Add(t),
        } => {
            assert_eq!(t.len(), 500, "should accept exactly 500 chars");
        }
        other => panic!("expected SystemAppend::Add for exactly 500 chars, got {other:?}"),
    }
}

/// Verify `/system +` (alias) also enforces the 500-char limit.
#[tokio::test]
async fn test_system_plus_exceeds_500_chars_rejected() {
    let sm = make_sm();
    let h = SystemHandler::new(Arc::clone(&sm));
    let content = "c".repeat(501);
    let args = format!("+ {content}");
    match h.handle(&args, &dummy_ctx()).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("超过 500 字符限制"),
                "+ alias should also enforce limit, got: {text}"
            );
        }
        other => panic!("expected Reply for + with >500 chars, got {other:?}"),
    }
}

/// Verify the error message format matches the plan spec.
#[tokio::test]
async fn test_system_add_error_message_format() {
    let sm = make_sm();
    let h = SystemHandler::new(Arc::clone(&sm));
    let content = "x".repeat(600);
    let args = format!("add {content}");
    match h.handle(&args, &dummy_ctx()).await {
        SlashResult::Reply(text) => {
            assert_eq!(
                text, "追加内容超过 500 字符限制，不截断。请精简后重试。",
                "error message should match plan spec"
            );
        }
        other => panic!("expected Reply, got {other:?}"),
    }
}

// ── Cross-step: /git all subcommands produce Exec with args ─────────────

use crate::handlers::WorkdirHandler;

/// Integration: all 5 supported git subcommands produce Exec results
/// with correct command strings, and the handler requires permission.
///
/// This verifies Step 1.5 implementation holistically: subcommand routing,
/// argument passthrough, and permission requirement.
#[tokio::test]
async fn test_cross_step_git_all_subcommands_with_args() {
    let sm = make_sm();
    let h = WorkdirHandler::new(sm);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    // All 5 subcommands should route to Exec.
    let subcommands_with_args = &[
        ("status", "git status"),
        ("log --oneline -10", "git log --oneline -10"),
        ("diff --staged", "git diff --staged"),
        ("branch -a", "git branch -a"),
        ("show HEAD", "git show HEAD"),
    ];
    for (args, expected) in subcommands_with_args {
        let result = h.handle(args, &ctx).await;
        match result {
            SlashResult::Exec {
                command,
                requires_permission,
            } => {
                assert_eq!(&command, expected, "for args: {args}");
                // Read-only subcommands should not require permission
                assert!(
                    !requires_permission,
                    "read-only subcommand '{args}' should not require permission"
                );
            }
            other => panic!("expected Exec for '{args}', got {other:?}"),
        }
    }
    // Handler-level requires_permission should be false (default).
    assert!(
        !h.requires_permission(),
        "WorkdirHandler should not require permission at handler level"
    );
}

// ── Cross-step: /git subcommand routes to Exec ─────────────────────────

/// Integration: all git subcommands (including write operations) route to Exec
/// so the Permission module can evaluate them.
#[tokio::test]
async fn test_cross_step_git_write_subcommand_routes_to_exec() {
    let sm = make_sm();
    let h = WorkdirHandler::new(sm);
    let ctx = SlashContext {
        command: "git".to_owned(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };
    // After Step 1.1, all subcommands route to Exec.
    match h.handle("push origin main", &ctx).await {
        SlashResult::Exec {
            command,
            requires_permission,
        } => {
            assert_eq!(command, "git push origin main");
            assert!(
                requires_permission,
                "write subcommand should require permission"
            );
        }
        other => panic!("expected Exec, got {other:?}"),
    }
}
