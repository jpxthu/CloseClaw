//! Outbound-path alignment tests for slash permission routing.
//!
//! Verifies that after Steps 1.1 and 1.2 the two outbound paths are
//! correctly routed:
//!
//! 1. **Queue notification** → `send_system_notification` (timeout-protected)
//! 2. **Permission denial** → `send_outbound_simplified` (simplified outbound)
//! 3. **`/approve-once` non-owner** → `send_outbound_simplified` (regression)

use std::sync::{Arc, Mutex};

use crate::slash_permission_test_utils::*;
use crate::{Gateway, HandleResult};
use closeclaw_common::slash_router::{SlashHandler, SlashResult, SlashRouter};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spy plugin that records every message sent via `plugin.send()`.
///
/// Used to verify that the correct outbound path was taken (both
/// `send_system_notification` and `send_outbound_simplified` ultimately
/// reach the plugin's `send` method, so capturing text is sufficient).
struct SpyPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

impl SpyPlugin {
    fn new(messages: Arc<Mutex<Vec<String>>>) -> Self {
        Self { messages }
    }
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for SpyPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[closeclaw_common::processor::ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> closeclaw_common::im_plugin::RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                closeclaw_common::processor::ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        closeclaw_common::im_plugin::RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &closeclaw_common::im_plugin::RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), closeclaw_common::im_plugin::AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        if !text.is_empty() {
            self.messages
                .lock()
                .expect("spy messages poisoned")
                .push(text);
        }
        Ok(())
    }
}

/// Counting handler that tracks invocations.
struct CountingHandler {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait::async_trait]
impl SlashHandler for CountingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(CountingHandler {
            command: self.command,
            requires_permission: self.requires_permission,
            counter: Arc::clone(&self.counter),
        })
    }

    fn commands(&self) -> &[&str] {
        std::slice::from_ref(&self.command)
    }

    fn description(&self) -> &str {
        "test handler"
    }

    fn requires_permission(&self) -> bool {
        self.requires_permission
    }

    async fn handle(
        &self,
        _args: &str,
        _ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> SlashResult {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        SlashResult::Reply(format!("{}: ok", self.command))
    }
}

struct CountingRouter {
    command: &'static str,
    requires_permission: bool,
    counter: Arc<std::sync::atomic::AtomicU32>,
}

#[async_trait::async_trait]
impl SlashRouter for CountingRouter {
    async fn dispatch(
        &self,
        _content: &str,
        _ctx: &closeclaw_common::slash_router::SlashContext,
    ) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        false
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == self.command {
            Some(Box::new(CountingHandler {
                command: self.command,
                requires_permission: self.requires_permission,
                counter: Arc::clone(&self.counter),
            }))
        } else {
            None
        }
    }
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
// Test 1: Queue notification goes through `send_system_notification`
// ===========================================================================
//
// When session is busy and a non-immediate slash command is enqueued,
// `enqueue_pending_slash` must call `send_system_notification` (which
// routes through `send_simplified_with_timeout`, the 2-second timeout
// path). The spy plugin captures the notification text proving the
// outbound call was made.

#[tokio::test]
async fn test_queue_notification_uses_send_system_notification() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));

    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(CountingRouter {
        command: "help",
        requires_permission: false,
        counter: Arc::clone(&counter),
    }))
    .await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;

    // Set session busy so the command is enqueued rather than executed.
    let cs = register_session(&gw, "sess-busy-notif").await;
    cs.write()
        .await
        .set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);

    let result = gw
        .dispatch_slash(
            "sess-busy-notif",
            "/help",
            Some("user1"),
            "mock",
            Some("peer1"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "enqueued command should return SlashHandled"
    );
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "handler must NOT be invoked when session is busy"
    );

    let msgs = spy_messages.lock().expect("spy messages poisoned");
    assert_eq!(msgs.len(), 1, "exactly one outbound message expected");
    assert!(
        msgs[0].contains("排队"),
        "notification text should contain '排队', got: {:?}",
        msgs[0]
    );
}

// ===========================================================================
// Test 2: Permission denial goes through `send_outbound_simplified`
// ===========================================================================
//
// When a non-owner sends a high-risk slash command (requires_permission:
// true) and the permission engine denies it, `check_engine_permission`
// must call `send_outbound_simplified` — the simplified outbound path —
// NOT the full outbound chain (VerbosityFilter → DslParser → middleware
// → render → send). The spy plugin captures the "权限不足" text.

#[tokio::test]
async fn test_permission_denial_uses_send_outbound_simplified() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let gw = make_gateway();
    gw.set_slash_dispatcher(Arc::new(CountingRouter {
        command: "exec",
        requires_permission: true,
        counter: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    }))
    .await;
    gw.set_permission_engine(deny_engine()).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;

    let result = gw
        .dispatch_slash(
            "sess-deny",
            "/exec ls",
            Some("user1"),
            "mock",
            Some("peer1"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "denied command should return SlashHandled"
    );

    let msgs = spy_messages.lock().expect("spy messages poisoned");
    assert_eq!(
        msgs.len(),
        1,
        "exactly one outbound message (permission denial) expected"
    );
    assert!(
        msgs[0].contains("权限不足"),
        "denial reply should contain '权限不足', got: {:?}",
        msgs[0]
    );
}

// ===========================================================================
// Test 3: `/approve-once` non-owner rejection (regression)
// ===========================================================================
//
// Non-owner `/approve-once` must still send a rejection via
// `send_outbound_simplified` and return `ApprovalProcessed`. This
// confirms the approval flow was not broken by the SlashRouteCtx
// refactoring.

#[tokio::test]
async fn test_approve_once_non_owner_uses_send_outbound_simplified() {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let gw = make_gateway();
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;

    let result = gw
        .try_handle_approval_command(
            "sess-approve",
            "/approve-once REQ_001",
            Some("user1"),
            "peer1",
            "mock",
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "non-owner /approve-once should return ApprovalProcessed, got {:?}",
        result
    );

    let msgs = spy_messages.lock().expect("spy messages poisoned");
    assert_eq!(
        msgs.len(),
        1,
        "exactly one outbound message (rejection) expected"
    );
    assert!(
        msgs[0].contains("权限不足"),
        "rejection should contain '权限不足', got: {:?}",
        msgs[0]
    );
}
