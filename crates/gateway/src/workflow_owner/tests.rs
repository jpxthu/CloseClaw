//! Unit tests for the gateway workflow entry point ② (workflow_owner.rs).
//!
//! Behavior dimensions under test (plan Step 1.5):
//! - owner non-slash message → `WorkflowControl::on_owner_message` invoked
//!   AND the message continues routing into the session handler;
//! - non-owner message → engine not called, message still routes;
//! - control unwired → silent skip, message still routes;
//! - engine error → logged, message still routes (non-blocking);
//! - legacy keyword messages ("恢复"/"终止") are no longer intercepted
//!   or consumed;
//! - `/workflow` non-owner → permission-denied reply, never enters the
//!   SlashDispatcher;
//! - `/workflow` owner → dispatched to the handler normally;
//! - non-`/workflow` slash commands pass the gate untouched.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::inbound_queue::InboundDebugCtx;
use crate::persist_test_utils::SnapshotMockPersist;
use crate::{Gateway, GatewayConfig, HandleResult, Session, SessionManager};
use closeclaw_common::im_plugin::{AdapterError, MessageType, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, ProcessedMessage};
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::{IMPlugin, WorkflowControl};
use closeclaw_session::llm_session::{ChatSession, ConversationSession};
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};

// ── Fake WorkflowControl ───────────────────────────────────────────────

/// Records `on_owner_message` invocations and returns a canned result.
struct FakeWorkflowControl {
    owner_calls: Mutex<Vec<String>>,
    owner_result: Result<(), String>,
}

impl FakeWorkflowControl {
    fn new(result: Result<(), String>) -> Self {
        Self {
            owner_calls: Mutex::new(Vec::new()),
            owner_result: result,
        }
    }

    fn owner_calls(&self) -> Vec<String> {
        self.owner_calls
            .lock()
            .expect("owner calls poisoned")
            .clone()
    }
}

#[async_trait]
impl WorkflowControl for FakeWorkflowControl {
    async fn start(&self, _session_id: &str, _workflow_name: &str) -> Result<(), String> {
        Ok(())
    }

    async fn on_owner_message(&self, session_id: &str) -> Result<(), String> {
        self.owner_calls
            .lock()
            .expect("owner calls poisoned")
            .push(session_id.to_owned());
        self.owner_result.clone()
    }

    async fn terminate(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }
}

// ── Spy plugin capturing outbound replies ──────────────────────────────

struct SpyPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

impl SpyPlugin {
    fn new(messages: Arc<Mutex<Vec<String>>>) -> Self {
        Self { messages }
    }
}

#[async_trait]
impl IMPlugin for SpyPlugin {
    fn platform(&self) -> &str {
        "mock"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        if !text.is_empty() {
            self.messages.lock().expect("spy poisoned").push(text);
        }
        Ok(())
    }
}

// ── Recording /workflow handler + router ───────────────────────────────

/// Handler recording `handle` invocations; replies with a text result.
struct RecordingHandler {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SlashHandler for RecordingHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(RecordingHandler {
            calls: Arc::clone(&self.calls),
        })
    }

    fn commands(&self) -> &[&str] {
        &["workflow"]
    }

    fn description(&self) -> &str {
        "recording workflow handler"
    }

    async fn handle(&self, args: &str, _ctx: &SlashContext) -> SlashResult {
        self.calls
            .lock()
            .expect("handler calls poisoned")
            .push(args.to_owned());
        SlashResult::Reply(format!("handled:{args}"))
    }
}

struct RecordingRouter {
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl SlashRouter for RecordingRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        false
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == "workflow" {
            Some(Box::new(RecordingHandler {
                calls: Arc::clone(&self.calls),
            }))
        } else {
            None
        }
    }
}

// ── Test environment ───────────────────────────────────────────────────

const SID: &str = "sid-owner";
const OWNER: &str = "owner-123";

struct TestEnv {
    gw: Arc<Gateway>,
    sm: Arc<SessionManager>,
    control: Option<Arc<FakeWorkflowControl>>,
    spy: Arc<Mutex<Vec<String>>>,
    handler_calls: Arc<Mutex<Vec<String>>>,
    _cooldown_dir: tempfile::TempDir,
}

impl TestEnv {
    fn owner_calls(&self) -> Vec<String> {
        self.control
            .as_ref()
            .map(|c| c.owner_calls())
            .unwrap_or_default()
    }

    fn handler_calls(&self) -> Vec<String> {
        self.handler_calls.lock().expect("calls poisoned").clone()
    }

    fn spy_texts(&self) -> Vec<String> {
        self.spy.lock().expect("spy poisoned").clone()
    }

    /// Text blocks currently present in the session transcript.
    async fn transcript_texts(&self) -> Vec<String> {
        let cs = self.sm.get_conversation_session(SID).await.unwrap();
        let cs = cs.read().await;
        cs.messages()
            .iter()
            .filter_map(|m| {
                m.content_blocks.iter().find_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.clone()),
                    _ => None,
                })
            })
            .collect()
    }
}

/// Full env: mock-persisted checkpoint (owner = `owner-123`), registered
/// session, conversation session, optional control, spy plugin, recording
/// router, and a session handler so non-slash messages flow through
/// `dispatch_to_handler`.
async fn setup_env(owner_result: Option<Result<(), String>>) -> TestEnv {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let persist = Arc::new(SnapshotMockPersist::new());
    let mut cp = SessionCheckpoint::new(SID.to_owned());
    cp.sender_id = Some(OWNER.to_owned());
    persist.save_checkpoint(&cp).await.unwrap();

    let sm = Arc::new(SessionManager::new(
        &config,
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        SID.to_owned(),
        Session {
            id: SID.to_owned(),
            agent_id: "test-agent".into(),
            channel: "mock".into(),
            created_at: 0,
            depth: 0,
        },
    );
    sm.conversation_sessions.write().await.insert(
        SID.to_owned(),
        Arc::new(RwLock::new(ConversationSession::new(
            SID.to_owned(),
            "model".to_owned(),
            std::env::temp_dir(),
        ))),
    );

    let control = owner_result.map(|r| Arc::new(FakeWorkflowControl::new(r)));
    if let Some(ctrl) = &control {
        sm.set_workflow_control(Arc::clone(ctrl) as Arc<dyn WorkflowControl>)
            .await;
    }

    let spy = Arc::new(Mutex::new(Vec::new()));
    let gw = Gateway::new_for_tests(config, Arc::clone(&sm));
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    let handler_calls = Arc::new(Mutex::new(Vec::new()));
    gw.set_slash_dispatcher(Arc::new(RecordingRouter {
        calls: Arc::clone(&handler_calls),
    }))
    .await;

    let cooldown_dir = tempfile::TempDir::new().expect("temp dir");
    let ufc = Arc::new(closeclaw_llm::unified_fallback::UnifiedFallbackClient::new(
        vec![],
        Arc::new(closeclaw_llm::retry::CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        )),
    ));
    let handler = Arc::new(
        crate::session_handler::SessionMessageHandler::new_no_output(
            Arc::clone(&sm),
            ufc,
            None,
            closeclaw_session::compaction::CompactConfig::default(),
        ),
    );
    let gw = Arc::new(gw.with_session_handler(handler));

    TestEnv {
        gw,
        sm,
        control,
        spy,
        handler_calls,
        _cooldown_dir: cooldown_dir,
    }
}

fn routed_msg(text: &str, peer: &str) -> ProcessedMessage {
    let mut m = std::collections::HashMap::new();
    m.insert("peer_id".into(), peer.into());
    m.insert(
        "message_type".into(),
        serde_json::to_string(&MessageType::Text).unwrap(),
    );
    m.insert("session_key".into(), "k".into());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(text.into())],
        metadata: m,
    }
}

fn empty_dbg() -> InboundDebugCtx<'static> {
    InboundDebugCtx {
        trace_id: None,
        session_key: None,
        root_ctx: None,
    }
}

/// Route a non-slash message through `route_and_dispatch`.
async fn route_non_slash(
    env: &TestEnv,
    content: &str,
    sender: Option<&str>,
) -> Option<HandleResult> {
    env.gw
        .route_and_dispatch(
            &routed_msg(content, "peer-owner"),
            SID,
            content.to_owned(),
            None,
            sender,
            "peer-owner",
            "mock",
            &empty_dbg(),
        )
        .await
}

/// Route a slash message through `route_and_dispatch`.
async fn route_slash(env: &TestEnv, content: &str, sender: Option<&str>) -> Option<HandleResult> {
    env.gw
        .route_and_dispatch(
            &routed_msg(content, "peer-owner"),
            SID,
            content.to_owned(),
            None,
            sender,
            "peer-owner",
            "mock",
            &empty_dbg(),
        )
        .await
}

// ── Tests: owner non-slash notification ───────────────────────────────

/// Owner non-slash message → `on_owner_message` called with the session
/// id, and the message continues routing (appended to the transcript).
#[tokio::test]
async fn test_owner_message_notifies_engine_and_continues_routing() {
    let env = setup_env(Some(Ok(()))).await;
    let result = route_non_slash(&env, "请继续", Some(OWNER)).await;

    assert_eq!(
        env.owner_calls(),
        vec![SID.to_owned()],
        "engine must be notified of the owner message"
    );
    // The message was not consumed: it flowed into the session handler
    // (which fails on the missing LLM caller after appending the message).
    assert!(
        matches!(result, Some(HandleResult::Error(_))),
        "message must continue routing, got {result:?}"
    );
    let texts = env.transcript_texts().await;
    assert!(
        texts.iter().any(|t| t.contains("请继续")),
        "message must reach the session transcript, got {texts:?}"
    );
}

/// Non-owner message → engine NOT called; message still routes.
#[tokio::test]
async fn test_non_owner_message_skips_engine_notification() {
    let env = setup_env(Some(Ok(()))).await;
    let result = route_non_slash(&env, "hello", Some("random-user")).await;

    assert!(
        env.owner_calls().is_empty(),
        "engine must not be notified for non-owner messages"
    );
    assert!(matches!(result, Some(HandleResult::Error(_))));
    let texts = env.transcript_texts().await;
    assert!(texts.iter().any(|t| t.contains("hello")));
}

/// Control unwired → silent skip; message still routes.
#[tokio::test]
async fn test_owner_message_unwired_control_silently_skips() {
    let env = setup_env(None).await;
    let result = route_non_slash(&env, "hello owner", Some(OWNER)).await;

    assert_eq!(env.owner_calls(), Vec::<String>::new());
    assert!(matches!(result, Some(HandleResult::Error(_))));
    let texts = env.transcript_texts().await;
    assert!(texts.iter().any(|t| t.contains("hello owner")));
}

/// Engine error → logged, message still routes (non-blocking).
#[tokio::test]
async fn test_engine_error_does_not_block_routing() {
    let env = setup_env(Some(Err("engine failure".to_owned()))).await;
    let result = route_non_slash(&env, "请继续", Some(OWNER)).await;

    assert_eq!(
        env.owner_calls(),
        vec![SID.to_owned()],
        "engine must be invoked even when it errors"
    );
    assert!(matches!(result, Some(HandleResult::Error(_))));
    let texts = env.transcript_texts().await;
    assert!(
        texts.iter().any(|t| t.contains("请继续")),
        "message must continue routing despite engine error"
    );
}

/// Legacy keyword messages ("恢复"/"终止") are no longer intercepted or
/// consumed: each one notifies the engine (if owner) and routes onward.
#[tokio::test]
async fn test_legacy_keywords_no_longer_intercepted() {
    let env = setup_env(Some(Ok(()))).await;
    for keyword in ["恢复", "终止"] {
        // Reset the idle state: the previous dispatch left the session in
        // Requesting (no LLM caller), which would queue the next message.
        let cs = env.sm.get_conversation_session(SID).await.unwrap();
        cs.write()
            .await
            .set_llm_state(closeclaw_common::LlmState::Idle);
        let result = route_non_slash(&env, keyword, Some(OWNER)).await;
        assert!(
            !matches!(
                result,
                Some(HandleResult::SlashHandled | HandleResult::ApprovalProcessed)
            ),
            "keyword message must not be consumed, got {result:?}"
        );
        assert!(
            matches!(result, Some(HandleResult::Error(_))),
            "keyword message must continue routing, got {result:?}"
        );
    }
    assert_eq!(
        env.owner_calls(),
        vec![SID.to_owned(), SID.to_owned()],
        "each keyword message must notify the engine"
    );
    let texts = env.transcript_texts().await;
    assert!(texts.iter().any(|t| t.contains("恢复")));
    assert!(texts.iter().any(|t| t.contains("终止")));
}

// ── Tests: /workflow gateway-layer hard intercept ──────────────────────

/// `/workflow` from a non-owner → permission-denied reply, handler never
/// invoked (does not enter the SlashDispatcher). `None` sender counts as
/// non-owner too.
#[tokio::test]
async fn test_workflow_non_owner_rejected_before_dispatcher() {
    let env = setup_env(Some(Ok(()))).await;
    for sender in [Some("random-user"), None] {
        let result = route_slash(&env, "/workflow deploy", sender).await;
        assert!(
            matches!(result, Some(HandleResult::SlashHandled)),
            "non-owner /workflow must be consumed at the gate, got {result:?}"
        );
        assert!(
            env.handler_calls().is_empty(),
            "handler must not be invoked for non-owner /workflow"
        );
        let texts = env.spy_texts();
        assert!(
            texts.iter().any(|t| t.contains("权限不足")),
            "permission-denied reply expected, got {texts:?}"
        );
    }
    assert!(
        env.owner_calls().is_empty(),
        "slash commands must not trigger owner-message notification"
    );
}

/// `/workflow` from the owner → dispatched to the handler normally.
#[tokio::test]
async fn test_workflow_owner_dispatched_to_handler() {
    let env = setup_env(Some(Ok(()))).await;
    let result = route_slash(&env, "/workflow deploy", Some("owner")).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "owner /workflow must be dispatched, got {result:?}"
    );
    assert_eq!(
        env.handler_calls(),
        vec!["deploy".to_owned()],
        "handler must receive the workflow name"
    );
    let texts = env.spy_texts();
    assert!(
        !texts.iter().any(|t| t.contains("权限不足")),
        "no permission-denied reply for owner, got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("handled:deploy")),
        "handler reply expected, got {texts:?}"
    );
}

/// Non-`/workflow` slash commands pass the gate untouched and reach the
/// SlashDispatcher (unknown command reply in this env).
#[tokio::test]
async fn test_non_workflow_slash_not_intercepted() {
    let env = setup_env(Some(Ok(()))).await;
    let result = route_slash(&env, "/status", Some("random-user")).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "unknown slash command must reach the dispatcher, got {result:?}"
    );
    assert!(env.handler_calls().is_empty());
    let texts = env.spy_texts();
    assert!(
        texts.iter().any(|t| t.contains("未知指令")),
        "dispatcher unknown-command reply expected, got {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("权限不足")),
        "non-/workflow commands must not be permission-blocked"
    );
}
