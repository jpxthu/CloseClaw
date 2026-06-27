//! Tests for `/system list` and `/system` (no args) branches.

use std::sync::Arc;

use crate::gateway::session_manager::SessionManager;
use crate::slash::context::SlashContext;
use crate::slash::handler::{SlashHandler, SlashResult};
use crate::slash::handlers::SystemHandler;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_sm() -> Arc<SessionManager> {
    use crate::gateway::DmScope;
    use closeclaw_session::bootstrap::loader::BootstrapMode;
    use closeclaw_session::persistence::ReasoningLevel;

    let gc = crate::gateway::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: DmScope::default(),
        ..Default::default()
    };
    Arc::new(SessionManager::new(
        &gc,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ))
}

async fn create_test_session(sm: &SessionManager) -> String {
    use crate::gateway::Message;

    let msg = Message {
        id: "sys-test-msg-1".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
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
            assert_eq!(text, "当前无追加指令", "got: {text}");
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
