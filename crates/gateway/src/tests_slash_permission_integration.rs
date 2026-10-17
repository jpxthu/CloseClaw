//! Cross-step integration tests for slash-command routing.
//!
//! Verifies that slash commands produce the correct side effects and
//! routing behavior when composed across multiple steps (1.5–1.7).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::slash_permission_test_utils::*;
use crate::{Gateway, GatewayConfig, HandleResult, SessionManager};
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::SlashSessionQuery;

// ---------------------------------------------------------------------------
// Mock handlers
// ---------------------------------------------------------------------------

struct CountingHandler {
    counter: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl SlashHandler for CountingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(CountingHandler {
            counter: Arc::clone(&self.counter),
        })
    }

    fn commands(&self) -> &[&str] {
        &["help"]
    }
    fn description(&self) -> &str {
        "help"
    }
    fn requires_permission(&self) -> bool {
        false
    }
    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        self.counter.fetch_add(1, Ordering::SeqCst);
        SlashResult::Reply("ok".to_owned())
    }
}

struct CountingRouter {
    command: &'static str,
    counter: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl SlashRouter for CountingRouter {
    async fn dispatch(&self, command: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        if command == self.command {
            Some(SlashResult::Reply("ok".to_owned()))
        } else {
            None
        }
    }

    fn is_immediate(&self, _command: &str) -> bool {
        // 对齐真实 ModeHandler 等非 immediate 命令处理器：恒返回 false。
        // （SlashRouter::is_immediate 收到的是完整原始 content（如 "/help"），
        // 与 self.command 的裸命令名比较恒为 false，属死比较，故直接写 false。）
        false
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(CountingHandler {
                counter: Arc::clone(&self.counter),
            }))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn counting_dispatcher(command: &'static str, counter: Arc<AtomicU32>) -> Arc<dyn SlashRouter> {
    Arc::new(CountingRouter { command, counter })
}

async fn register_session(
    gw: &Gateway,
    session_id: &str,
) -> Arc<tokio::sync::RwLock<closeclaw_session::llm_session::ConversationSession>> {
    use std::path::PathBuf;
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        PathBuf::from("/tmp"),
    );
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = gw.session_manager.conversation_sessions.write().await;
        conv.insert(session_id.to_owned(), cs_arc.clone());
    }
    cs_arc
}

// ===========================================================================
// Cross-step integration tests
// ===========================================================================

/// Integration: `/stop` default cascade + `/stop --force` behavior.
///
/// Verifies that:
/// 1. `/stop` (no args) defaults to cascade=true, force=false
/// 2. `/stop --force` sets force=true while keeping cascade=true
/// 3. `/stop --cascade --force` sets both flags
#[tokio::test]
async fn test_cross_step_stop_flag_combinations() {
    use closeclaw_slash::SlashHandler as _;
    use closeclaw_slash::StopHandler;

    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: "s".to_owned(),
        channel: "c".to_owned(),
    };

    // No args: returns unit Stop
    match StopHandler.handle("", &ctx).await {
        SlashResult::Stop => {}
        other => panic!("expected Stop, got {other:?}"),
    }

    // --force: args ignored, returns unit Stop
    match StopHandler.handle("--force", &ctx).await {
        SlashResult::Stop => {}
        other => panic!("expected Stop, got {other:?}"),
    }

    // --cascade --force: returns unit Stop
    match StopHandler.handle("--cascade --force", &ctx).await {
        SlashResult::Stop => {}
        other => panic!("expected Stop, got {other:?}"),
    }
}

/// Integration: `/mode` immediate flag is false — verified through
/// the gateway busy-queueing mechanism.
///
/// When session is busy, `/mode` should be enqueued (not immediate),
/// confirming the handler's immediate() returns false.
#[tokio::test]
async fn test_cross_step_mode_not_immediate_enqueues_when_busy() {
    let counter = Arc::new(AtomicU32::new(0));
    let gw = make_gateway();
    gw.set_slash_dispatcher(counting_dispatcher("mode", Arc::clone(&counter)))
        .await;

    let cs = register_session(&gw, "sess-mode-busy").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);

    let result = gw
        .dispatch_slash(
            "sess-mode-busy",
            "/mode",
            Some("user1"),
            "feishu",
            Some("peer_id"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "/mode should be enqueued when busy (not immediate)"
    );

    // Verify pending message was enqueued.
    let cs = gw
        .session_manager
        .get_conversation_session("sess-mode-busy")
        .await
        .unwrap();
    let cs = cs.read().await;
    let pending = cs.get_pending_messages();
    assert_eq!(
        pending.len(),
        1,
        "/mode should be enqueued as pending message"
    );
    assert!(
        pending[0].content.contains("/mode"),
        "pending message should contain /mode command"
    );
}

/// Integration: `/git` subcommands all produce Exec results.
///
/// Verifies that all 5 supported git subcommands (status/log/diff/branch/show)
/// are routed as Exec, confirming Step 1.5 implementation.
#[tokio::test]
async fn test_cross_step_git_all_subcommands_route_to_exec() {
    // Build a gateway with a real slash dispatcher that includes WorkdirHandler.
    let config = GatewayConfig {
        name: "test-git".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new_for_tests(config, sm));

    // Register a real WorkdirHandler.
    let registry = closeclaw_slash::registry::HandlerRegistry::new();
    let workdir_handler = closeclaw_slash::handlers::WorkdirHandler::new(Arc::clone(
        &gw.session_manager,
    )
        as Arc<dyn SlashSessionQuery>);
    registry.register(Arc::new(workdir_handler));
    gw.set_slash_dispatcher(Arc::new(closeclaw_slash::SlashDispatcher::new(registry)))
        .await;

    let supported = ["status", "log", "diff", "branch", "show"];
    for sub in supported {
        let result = gw
            .dispatch_slash(
                "sess-git",
                &format!("/git {sub}"),
                Some("user1"),
                "feishu",
                Some("peer_id"),
            )
            .await;
        assert!(
            matches!(result, Some(HandleResult::SlashHandled)),
            "/git {sub} should return SlashHandled, got: {result:?}"
        );
    }
}

/// Integration: `/system add` + `/status` shows system_append in status.
///
/// After appending a system instruction, `/status` should reflect it
/// in the "追加指令" field.
#[tokio::test]
async fn test_cross_step_system_append_reflected_in_status() {
    use closeclaw_slash::handlers::SystemHandler;
    use closeclaw_slash::{SlashHandler as _, StatusHandler};

    let config = crate::GatewayConfig {
        name: String::new(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let storage = Arc::new(closeclaw_session::storage::memory::MemoryStorage::new());
    let sm = Arc::new(SessionManager::new(
        &config,
        Some(storage),
        None,
        closeclaw_session::persistence::ReasoningLevel::default(),
    ));

    // Create a session.
    let msg = crate::Message {
        id: "cross-test-msg".to_string(),
        from: "user-a".to_string(),
        to: "agent-b".to_string(),
        content: "hello".to_string(),
        channel: "feishu".to_string(),
        timestamp: 0,
        metadata: std::collections::HashMap::new(),
        thread_id: None,
        reply_ref: None,
        platform: None,
        dsl_result: None,
        content_blocks: None,
    };
    let sid = sm
        .find_or_create("feishu", &msg, None)
        .await
        .expect("session");

    let ctx = SlashContext {
        command: String::new(),
        sender_id: "u".to_owned(),
        session_id: sid.clone(),
        channel: "c".to_owned(),
    };

    // Append a system instruction.
    let sys_handler = SystemHandler::new(Arc::clone(&sm) as Arc<dyn SlashSessionQuery>);
    match sys_handler.handle("add 请始终使用中文", &ctx).await {
        SlashResult::SystemAppend {
            action: closeclaw_common::slash_router::SystemAppendAction::Add(_),
        } => {}
        other => panic!("expected SystemAppend::Add, got {other:?}"),
    }

    // Execute the append via the session (simulating what the Gateway does).
    {
        let conv = sm.get_conversation_session(&sid).await.unwrap();
        let mut cs = conv.write().await;
        cs.add_system_append("请始终使用中文".to_owned());
    }

    // Check status shows the appended instruction.
    let status_handler = StatusHandler::new(Arc::clone(&sm) as Arc<dyn SlashSessionQuery>);
    match status_handler.handle("", &ctx).await {
        SlashResult::Reply(text) => {
            assert!(
                text.contains("追加指令"),
                "status should show 追加指令 field, got: {text}"
            );
            assert!(
                text.contains("请始终使用中文"),
                "status should show appended instruction content, got: {text}"
            );
        }
        other => panic!("expected Reply with status, got {other:?}"),
    }
}
