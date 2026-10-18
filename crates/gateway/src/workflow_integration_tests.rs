//! Integration tests (gateway in-src): WorkflowControl end-to-end wiring.
//!
//! Drives the full in-process chain through the gateway inbound router
//! (`route_and_dispatch`) with the real slash dispatcher (`StopHandler`)
//! and the real workflow `WorkflowHandler`, a fake `WorkflowControl`, a
//! spy IM plugin, and mock checkpoint persistence. No real LLM / network.
//!
//! Scenarios (plan Step 1.8):
//! 1. Owner `/workflow` full chain: gateway gate → dispatcher →
//!    `WorkflowHandler` → `SlashResult::WorkflowStart` →
//!    `execute_workflow_start` → `WorkflowControl::start` → confirmation
//!    reply.
//! 2. Owner non-slash message on a blocked session: `on_owner_message`
//!    invoked, message continues routing into the LLM handler.
//! 3. Owner `/stop`: stop completes → `terminate` called.
//! 4. Non-owner `/stop`: stop runs but the workflow is NOT terminated
//!    (permission boundary).
//! 5. Idempotency: repeated owner `/stop` terminates repeatedly without
//!    error.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::gateway_test_utils::{empty_dbg, routed_msg, SpyPlugin};
use crate::persist_test_utils::SnapshotMockPersist;
use crate::{Gateway, GatewayConfig, HandleResult, Session, SessionManager};
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::WorkflowControl;
use closeclaw_session::llm_session::{ChatSession, ConversationSession};
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};
use closeclaw_slash::dispatcher::SlashDispatcher;
use closeclaw_slash::registry::HandlerRegistry;
use closeclaw_slash::StopHandler;
use closeclaw_workflow::handler::WorkflowHandler;
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};

// ── Fake WorkflowControl ───────────────────────────────────────────────

/// Records all three `WorkflowControl` entry points and returns canned
/// results (start errors are configurable per test).
struct FakeWorkflowControl {
    start_calls: Mutex<Vec<(String, String)>>,
    owner_calls: Mutex<Vec<String>>,
    terminate_calls: Mutex<Vec<String>>,
    start_result: Result<(), String>,
}

impl FakeWorkflowControl {
    fn new(start_result: Result<(), String>) -> Self {
        Self {
            start_calls: Mutex::new(Vec::new()),
            owner_calls: Mutex::new(Vec::new()),
            terminate_calls: Mutex::new(Vec::new()),
            start_result,
        }
    }

    fn start_calls(&self) -> Vec<(String, String)> {
        self.start_calls
            .lock()
            .expect("start calls poisoned")
            .clone()
    }

    fn owner_calls(&self) -> Vec<String> {
        self.owner_calls
            .lock()
            .expect("owner calls poisoned")
            .clone()
    }

    fn terminate_calls(&self) -> Vec<String> {
        self.terminate_calls
            .lock()
            .expect("terminate calls poisoned")
            .clone()
    }
}

#[async_trait]
impl WorkflowControl for FakeWorkflowControl {
    async fn start(&self, session_id: &str, workflow_name: &str) -> Result<(), String> {
        self.start_calls
            .lock()
            .expect("start calls poisoned")
            .push((session_id.to_owned(), workflow_name.to_owned()));
        self.start_result.clone()
    }

    async fn on_owner_message(&self, session_id: &str) -> Result<(), String> {
        self.owner_calls
            .lock()
            .expect("owner calls poisoned")
            .push(session_id.to_owned());
        Ok(())
    }

    async fn terminate(&self, session_id: &str) -> Result<(), String> {
        self.terminate_calls
            .lock()
            .expect("terminate calls poisoned")
            .push(session_id.to_owned());
        Ok(())
    }
}

// ── Test environment ───────────────────────────────────────────────────

const SID: &str = "sid-int";
const OWNER: &str = "owner";

struct TestEnv {
    gw: Arc<Gateway>,
    sm: Arc<SessionManager>,
    control: Arc<FakeWorkflowControl>,
    spy: Arc<Mutex<Vec<String>>>,
    _cooldown_dir: tempfile::TempDir,
}

impl TestEnv {
    fn spy_texts(&self) -> Vec<String> {
        self.spy.lock().expect("spy poisoned").clone()
    }

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

    /// Phase of the type-erased workflow run stored on the session.
    async fn stored_run_phase(&self) -> Option<Phase> {
        let cs = self.sm.get_conversation_session(SID).await.unwrap();
        let value = cs.read().await.workflow_run_value()?;
        serde_json::from_value::<WorkflowRun>(value)
            .ok()
            .map(|r| r.phase)
    }
}

/// A workflow run parked in the `Blocked` phase (verify retry limit hit).
fn blocked_run() -> WorkflowRun {
    WorkflowRun {
        workflow_id: "wf-int-1".to_owned(),
        definition_name: "deploy".to_owned(),
        definition_version: "1.0.0".to_owned(),
        current_step: 1,
        phase: Phase::Blocked,
        current_step_entered_at: "2026-10-19T00:00:00Z".to_owned(),
        step_history: Vec::new(),
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::Normal,
        pending_verify: PendingVerify::default(),
        paused_reason: "verify retry limit exceeded".to_owned(),
    }
}

/// Full env: mock-persisted checkpoint (owner = `owner`), registered
/// session + conversation session, wired fake control, spy plugin, the
/// real slash dispatcher (`/stop` + `/workflow` handlers), and a
/// no-output session handler so non-slash messages route onward.
async fn setup_env(start_result: Result<(), String>) -> TestEnv {
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
            agent_id: "peer-int".into(),
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

    let control = Arc::new(FakeWorkflowControl::new(start_result));
    sm.set_workflow_control(Arc::clone(&control) as Arc<dyn WorkflowControl>)
        .await;

    let spy = Arc::new(Mutex::new(Vec::new()));
    let registry = Arc::new(HandlerRegistry::new());
    registry.register(Arc::new(StopHandler));
    registry.register(Arc::new(WorkflowHandler::new()));
    let gw = Gateway::new_for_tests(config, Arc::clone(&sm));
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    gw.set_slash_dispatcher(Arc::new(SlashDispatcher::from_shared(registry)))
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
        _cooldown_dir: cooldown_dir,
    }
}

/// Route a message (slash or non-slash) through `route_and_dispatch`.
async fn route(env: &TestEnv, content: &str, sender: Option<&str>) -> Option<HandleResult> {
    env.gw
        .route_and_dispatch(
            &routed_msg(content, "peer-int"),
            SID,
            content.to_owned(),
            None,
            sender,
            "peer-int",
            "mock",
            &empty_dbg(),
        )
        .await
}

// ── Tests: owner /workflow full chain ──────────────────────────────────

/// Owner `/workflow` full chain: the real `WorkflowHandler` parses the
/// name, the gateway executor starts it via `WorkflowControl::start`, and
/// the confirmation reply goes out. No other entry point fires.
#[tokio::test]
async fn test_owner_workflow_full_chain_starts_and_replies() {
    let env = setup_env(Ok(())).await;

    let result = route(&env, "/workflow deploy", Some(OWNER)).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "owner /workflow must be dispatched, got {result:?}"
    );
    assert_eq!(
        env.control.start_calls(),
        vec![(SID.to_owned(), "deploy".to_owned())],
        "WorkflowControl::start must run with the session id and workflow name"
    );
    assert!(
        env.control.owner_calls().is_empty(),
        "/workflow must not trigger the owner-message entry"
    );
    assert!(
        env.control.terminate_calls().is_empty(),
        "/workflow must not terminate anything"
    );
    let texts = env.spy_texts();
    assert_eq!(texts.len(), 1, "exactly one reply expected, got {texts:?}");
    assert!(
        texts[0].contains("已启动 workflow：deploy"),
        "confirmation reply expected, got: {:?}",
        texts[0]
    );
}

/// Owner `/workflow` with a failing engine: `start` still runs once and
/// the error reply carries the workflow name and the failure reason.
#[tokio::test]
async fn test_owner_workflow_start_err_replies_error() {
    let env = setup_env(Err("定义不存在".to_owned())).await;

    let result = route(&env, "/workflow deploy", Some(OWNER)).await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        env.control.start_calls().len(),
        1,
        "start must be attempted exactly once"
    );
    let texts = env.spy_texts();
    assert_eq!(texts.len(), 1, "exactly one reply expected, got {texts:?}");
    assert!(
        texts[0].contains("deploy")
            && texts[0].contains("启动失败")
            && texts[0].contains("定义不存在"),
        "error reply must carry name and reason, got: {:?}",
        texts[0]
    );
}

// ── Tests: owner non-slash message on a blocked session ────────────────

/// Owner non-slash message while the session's workflow run is Blocked:
/// `on_owner_message` fires and the message continues routing into the
/// LLM handler (appended to the transcript); the run state is untouched
/// by the gateway (resolution is the engine's job).
#[tokio::test]
async fn test_owner_non_slash_on_blocked_session_notifies_and_routes() {
    let env = setup_env(Ok(())).await;
    {
        let cs = env.sm.get_conversation_session(SID).await.unwrap();
        let run = serde_json::to_value(blocked_run()).expect("blocked run encodes");
        cs.write().await.set_workflow_run_value(Some(run));
    }

    let result = route(&env, "请继续", Some(OWNER)).await;

    assert_eq!(
        env.control.owner_calls(),
        vec![SID.to_owned()],
        "engine must be notified of the owner message"
    );
    assert!(
        matches!(result, Some(HandleResult::Error(_))),
        "message must continue routing into the session handler, got {result:?}"
    );
    let texts = env.transcript_texts().await;
    assert!(
        texts.iter().any(|t| t.contains("请继续")),
        "message must reach the session transcript, got {texts:?}"
    );
    assert_eq!(
        env.stored_run_phase().await,
        Some(Phase::Blocked),
        "gateway must not touch the blocked run state"
    );
    assert!(env.control.start_calls().is_empty());
    assert!(env.control.terminate_calls().is_empty());
}

// ── Tests: owner /stop → terminate ─────────────────────────────────────

/// Owner `/stop`: the stop flow completes, then `terminate` runs for the
/// session and the stop confirmation goes out.
#[tokio::test]
async fn test_owner_stop_terminates_workflow() {
    let env = setup_env(Ok(())).await;

    let result = route(&env, "/stop", Some(OWNER)).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "owner /stop must be handled, got {result:?}"
    );
    assert_eq!(
        env.control.terminate_calls(),
        vec![SID.to_owned()],
        "owner /stop must terminate the session's workflow"
    );
    assert!(env.control.start_calls().is_empty());
    let texts = env.spy_texts();
    assert!(
        texts.iter().any(|t| t.contains("已停止当前任务")),
        "stop confirmation reply expected, got {texts:?}"
    );
}

/// Non-owner `/stop` (permission boundary): the stop itself is allowed
/// and replies, but the workflow is NOT terminated.
#[tokio::test]
async fn test_non_owner_stop_does_not_terminate() {
    let env = setup_env(Ok(())).await;

    let result = route(&env, "/stop", Some("intruder")).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "non-owner /stop must still be handled, got {result:?}"
    );
    assert!(
        env.control.terminate_calls().is_empty(),
        "non-owner /stop must not terminate the workflow"
    );
    let texts = env.spy_texts();
    assert!(
        texts.iter().any(|t| t.contains("已停止当前任务")),
        "stop confirmation reply expected for non-owner too, got {texts:?}"
    );
}

/// Repeated owner `/stop` (idempotency): each stop terminates again
/// without error and each confirmation goes out.
#[tokio::test]
async fn test_repeated_owner_stop_terminates_idempotently() {
    let env = setup_env(Ok(())).await;

    for i in 1..=2 {
        let result = route(&env, "/stop", Some(OWNER)).await;
        assert!(
            matches!(result, Some(HandleResult::SlashHandled)),
            "stop #{i} must be handled, got {result:?}"
        );
    }

    assert_eq!(
        env.control.terminate_calls(),
        vec![SID.to_owned(), SID.to_owned()],
        "each owner /stop must terminate (idempotent repeat)"
    );
    let texts = env.spy_texts();
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.contains("已停止当前任务"))
            .count(),
        2,
        "each stop must confirm, got {texts:?}"
    );
}
