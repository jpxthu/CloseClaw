//! Tests for gateway entry ③: owner `/stop` terminates the workflow.
//!
//! Verifies the `/stop` side-effect chain (`SlashResult::Stop` →
//! `GatewaySlashExecutor::execute_stop` → `gw_stop`) terminating the
//! session's active workflow via the wired
//! [`WorkflowControl`](closeclaw_common::WorkflowControl):
//!
//! 1. **Owner + stop completed** → `terminate` called with the session id;
//!    confirmation reply sent through the outbound chain.
//! 2. **Non-owner (or unknown sender)** → stop runs, no `terminate` call.
//! 3. **Control unwired** → silent skip; stop reply still sent.
//! 4. **Graceful timeout fallback** → after the forceful escalation,
//!    `terminate` is still called.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::gateway_test_utils::SpyPlugin;
use crate::persist_test_utils::SnapshotMockPersist;
use crate::{Gateway, GatewayConfig, HandleResult, Session, SessionManager};
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::{ToolExecState, WorkflowControl};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::{PersistenceService, ReasoningLevel, SessionCheckpoint};

// ── Fake WorkflowControl ───────────────────────────────────────────────

/// Records `terminate` invocations and returns a canned result.
struct FakeWorkflowControl {
    terminate_calls: Mutex<Vec<String>>,
    result: Result<(), String>,
}

impl FakeWorkflowControl {
    fn new(result: Result<(), String>) -> Self {
        Self {
            terminate_calls: Mutex::new(Vec::new()),
            result,
        }
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
    async fn start(&self, _session_id: &str, _workflow_name: &str) -> Result<(), String> {
        Ok(())
    }

    async fn on_owner_message(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn terminate(&self, session_id: &str) -> Result<(), String> {
        self.terminate_calls
            .lock()
            .expect("terminate calls poisoned")
            .push(session_id.to_owned());
        self.result.clone()
    }
}

// ── Router returning SlashResult::Stop ─────────────────────────────────

struct StopHandler;

#[async_trait]
impl SlashHandler for StopHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(StopHandler)
    }

    fn commands(&self) -> &[&str] {
        &["stop"]
    }

    fn description(&self) -> &str {
        "stop handler"
    }

    fn immediate(&self, _cmd: &str, _args: &str) -> bool {
        true
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::Stop
    }
}

struct StopRouter;

#[async_trait]
impl SlashRouter for StopRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        true
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == "stop" {
            Some(Box::new(StopHandler))
        } else {
            None
        }
    }
}

// ── Test environment ───────────────────────────────────────────────────

const SID: &str = "sid-stop";
const OWNER: &str = "owner-123";

struct TestEnv {
    gw: Arc<Gateway>,
    sm: Arc<SessionManager>,
    spy: Arc<Mutex<Vec<String>>>,
}

impl TestEnv {
    fn spy_texts(&self) -> Vec<String> {
        self.spy.lock().expect("spy poisoned").clone()
    }
}

/// Env: mock-persisted checkpoint (owner = `owner-123`), registered
/// session + conversation session, optional control, spy plugin, and a
/// `/stop` dispatcher.
async fn setup_env(control: Option<Arc<FakeWorkflowControl>>) -> TestEnv {
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
            agent_id: "peer-stop".into(),
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

    if let Some(ctrl) = &control {
        sm.set_workflow_control(Arc::clone(ctrl) as Arc<dyn WorkflowControl>)
            .await;
    }

    let spy = Arc::new(Mutex::new(Vec::new()));
    let gw = Arc::new(Gateway::new_for_tests(config, Arc::clone(&sm)));
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    gw.set_slash_dispatcher(Arc::new(StopRouter)).await;

    TestEnv { gw, sm, spy }
}

/// Dispatch `/stop` through the gateway slash entry.
async fn dispatch_stop(gw: &Gateway, sender: Option<&str>) -> Option<HandleResult> {
    gw.dispatch_slash(SID, "/stop", sender, "mock", Some("peer-stop"))
        .await
}

// ── Tests ──────────────────────────────────────────────────────────────

/// Owner `/stop` → stop completes → `terminate` called with the session
/// id; confirmation reply sent.
#[tokio::test]
async fn test_owner_stop_terminates_workflow() {
    let control = Arc::new(FakeWorkflowControl::new(Ok(())));
    let env = setup_env(Some(Arc::clone(&control))).await;

    let result = dispatch_stop(&env.gw, Some(OWNER)).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "stop must be handled, got {result:?}"
    );
    assert_eq!(
        control.terminate_calls(),
        vec![SID.to_owned()],
        "owner /stop must terminate the session's workflow"
    );
    let texts = env.spy_texts();
    assert!(
        texts.iter().any(|t| t.contains("已停止当前任务")),
        "stop confirmation reply expected, got {texts:?}"
    );
}

/// Non-owner (or unknown sender) `/stop` → stop runs and replies, but
/// the workflow is NOT terminated.
#[tokio::test]
async fn test_non_owner_stop_skips_terminate() {
    let control = Arc::new(FakeWorkflowControl::new(Ok(())));
    let env = setup_env(Some(Arc::clone(&control))).await;

    for sender in [Some("random-user"), None] {
        let result = dispatch_stop(&env.gw, sender).await;
        assert!(
            matches!(result, Some(HandleResult::SlashHandled)),
            "stop must be handled for sender {sender:?}, got {result:?}"
        );
    }

    assert!(
        control.terminate_calls().is_empty(),
        "non-owner /stop must not terminate the workflow"
    );
    let texts = env.spy_texts();
    assert_eq!(
        texts.len(),
        2,
        "each stop still replies with confirmation, got {texts:?}"
    );
}

/// Control unwired → silent skip; the stop itself still runs and replies.
#[tokio::test]
async fn test_unwired_control_skips_terminate() {
    let env = setup_env(None).await;

    let result = dispatch_stop(&env.gw, Some(OWNER)).await;

    assert!(
        matches!(result, Some(HandleResult::SlashHandled)),
        "stop must be handled, got {result:?}"
    );
    let texts = env.spy_texts();
    assert!(
        texts.iter().any(|t| t.contains("已停止当前任务")),
        "stop reply must still be sent without control, got {texts:?}"
    );
}

/// Graceful timeout → forceful escalation fallback: once the fallback
/// force stop completes, `terminate` is still called for the owner.
#[tokio::test]
async fn test_timed_out_fallback_still_terminates() {
    let control = Arc::new(FakeWorkflowControl::new(Ok(())));
    let env = setup_env(Some(Arc::clone(&control))).await;

    // Block the graceful wait with a never-finishing foreground tool so
    // the stop takes the TimedOut → forceful escalation path.
    let cs = env.sm.get_conversation_session(SID).await.unwrap();
    {
        let guard = cs.read().await;
        guard.tool_states.write().expect("tool_states lock").insert(
            "blocker-tool".into(),
            (ToolExecState::RunningForeground, None),
        );
    }

    crate::slash_executor_helpers::gw_stop(
        &env.sm,
        SID,
        true,
        true,
        Some(OWNER),
        Duration::from_millis(50),
    )
    .await;

    assert!(
        cs.read().await.is_cancelled(),
        "fallback must have force-stopped the session"
    );
    assert_eq!(
        control.terminate_calls(),
        vec![SID.to_owned()],
        "terminate must run after the TimedOut forceful fallback"
    );
}
