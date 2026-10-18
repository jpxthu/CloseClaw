//! Tests for `GatewaySlashExecutor::execute_workflow_start` (entry point ①).
//!
//! Verifies the gateway-side wiring of `SlashResult::WorkflowStart`:
//!
//! 1. **Wired + Ok** → `WorkflowControl::start` called with the session ID
//!    and workflow name; confirmation reply sent through the outbound chain.
//! 2. **Wired + Err** → `start` called; error reply carries the workflow
//!    name and the failure reason.
//! 3. **Not wired** → no engine call; error reply carries the workflow name.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::slash_permission_test_utils::*;
use crate::{Gateway, HandleResult, Session, SessionManager};
use closeclaw_common::executor::SlashEffectExecutor;
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::WorkflowControl;
use closeclaw_session::persistence::ReasoningLevel;

// ---------------------------------------------------------------------------
// Fake WorkflowControl
// ---------------------------------------------------------------------------

/// Records `start` invocations and returns a canned result.
struct FakeWorkflowControl {
    calls: Mutex<Vec<(String, String)>>,
    result: Result<(), String>,
}

impl FakeWorkflowControl {
    fn new(result: Result<(), String>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            result,
        }
    }

    fn start_calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("calls poisoned").clone()
    }
}

#[async_trait]
impl WorkflowControl for FakeWorkflowControl {
    async fn start(&self, session_id: &str, workflow_name: &str) -> Result<(), String> {
        self.calls
            .lock()
            .expect("calls poisoned")
            .push((session_id.to_owned(), workflow_name.to_owned()));
        self.result.clone()
    }

    async fn on_owner_message(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn terminate(&self, _session_id: &str) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Spy plugin capturing outbound replies
// ---------------------------------------------------------------------------

struct SpyPlugin {
    messages: Arc<Mutex<Vec<String>>>,
}

impl SpyPlugin {
    fn new(messages: Arc<Mutex<Vec<String>>>) -> Self {
        Self { messages }
    }
}

#[async_trait]
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

// ---------------------------------------------------------------------------
// Router returning SlashResult::WorkflowStart
// ---------------------------------------------------------------------------

struct WorkflowStartHandler {
    name: &'static str,
}

#[async_trait]
impl SlashHandler for WorkflowStartHandler {
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(WorkflowStartHandler { name: self.name })
    }

    fn commands(&self) -> &[&str] {
        &["workflow"]
    }

    fn description(&self) -> &str {
        "workflow start handler"
    }

    async fn handle(&self, _args: &str, _ctx: &SlashContext) -> SlashResult {
        SlashResult::WorkflowStart {
            name: self.name.to_owned(),
        }
    }
}

struct WorkflowStartRouter {
    name: &'static str,
}

#[async_trait]
impl SlashRouter for WorkflowStartRouter {
    async fn dispatch(&self, _content: &str, _ctx: &SlashContext) -> Option<SlashResult> {
        None
    }

    fn is_immediate(&self, _command: &str) -> bool {
        false
    }

    fn get_handler(&self, command: &str) -> Option<Box<dyn SlashHandler>> {
        if command == "workflow" {
            Some(Box::new(WorkflowStartHandler { name: self.name }))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Set up a gateway with a registered session, a spy plugin, a
/// `/workflow` router, and (optionally) a wired workflow control.
async fn setup(
    control: Option<Arc<FakeWorkflowControl>>,
) -> (Arc<Gateway>, Arc<Mutex<Vec<String>>>) {
    let spy_messages: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let config = crate::GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let session_id = "sess-wf";

    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.to_owned(),
        "test-model".to_owned(),
        std::path::PathBuf::from("/tmp"),
    );
    sm.conversation_sessions.write().await.insert(
        session_id.to_owned(),
        Arc::new(tokio::sync::RwLock::new(cs)),
    );
    sm.sessions.write().await.insert(
        session_id.to_owned(),
        Session {
            id: session_id.to_owned(),
            agent_id: "peer-wf".to_owned(),
            channel: "mock".to_owned(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );

    let gw = Arc::new(Gateway::new_for_tests(config, sm));
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy_messages))))
        .await;
    gw.set_slash_dispatcher(Arc::new(WorkflowStartRouter { name: "deploy" }))
        .await;
    if let Some(ctrl) = control {
        gw.session_manager
            .set_workflow_control(ctrl as Arc<dyn WorkflowControl>)
            .await;
    }
    (gw, spy_messages)
}

fn spy_texts(spy: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    spy.lock().expect("spy messages poisoned").clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Wired + Ok: `start` called with (session_id, name); confirmation reply.
#[tokio::test]
async fn test_wired_start_ok_calls_start_and_replies_confirm() {
    let control = Arc::new(FakeWorkflowControl::new(Ok(())));
    let (gw, spy) = setup(Some(Arc::clone(&control))).await;

    let result = gw
        .dispatch_slash(
            "sess-wf",
            "/workflow deploy",
            Some("owner"),
            "mock",
            Some("peer-wf"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let calls = control.start_calls();
    assert_eq!(
        calls,
        vec![("sess-wf".to_owned(), "deploy".to_owned())],
        "WorkflowControl::start must be called with session ID and workflow name"
    );
    let msgs = spy_texts(&spy);
    assert_eq!(msgs.len(), 1, "exactly one reply expected, got {msgs:?}");
    assert!(
        msgs[0].contains("已启动 workflow：deploy"),
        "confirmation reply expected, got: {:?}",
        msgs[0]
    );
}

/// Wired + Err: `start` called; error reply carries name and reason.
#[tokio::test]
async fn test_wired_start_err_replies_error_with_name_and_reason() {
    let control = Arc::new(FakeWorkflowControl::new(Err("定义不存在".to_owned())));
    let (gw, spy) = setup(Some(Arc::clone(&control))).await;

    let result = gw
        .dispatch_slash(
            "sess-wf",
            "/workflow deploy",
            Some("owner"),
            "mock",
            Some("peer-wf"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    assert_eq!(
        control.start_calls().len(),
        1,
        "WorkflowControl::start must be called exactly once"
    );
    let msgs = spy_texts(&spy);
    assert_eq!(msgs.len(), 1, "exactly one reply expected, got {msgs:?}");
    assert!(
        msgs[0].contains("deploy"),
        "error reply must carry the workflow name, got: {:?}",
        msgs[0]
    );
    assert!(
        msgs[0].contains("启动失败"),
        "error reply must mark the failure, got: {:?}",
        msgs[0]
    );
    assert!(
        msgs[0].contains("定义不存在"),
        "error reply must carry the failure reason, got: {:?}",
        msgs[0]
    );
}

/// Not wired: no engine call; error reply carries the workflow name.
#[tokio::test]
async fn test_not_wired_replies_error_without_engine_call() {
    let (gw, spy) = setup(None).await;

    let result = gw
        .dispatch_slash(
            "sess-wf",
            "/workflow deploy",
            Some("owner"),
            "mock",
            Some("peer-wf"),
        )
        .await;

    assert!(matches!(result, Some(HandleResult::SlashHandled)));
    let msgs = spy_texts(&spy);
    assert_eq!(msgs.len(), 1, "exactly one reply expected, got {msgs:?}");
    assert!(
        msgs[0].contains("deploy"),
        "error reply must carry the workflow name, got: {:?}",
        msgs[0]
    );
    assert!(
        msgs[0].contains("未配置"),
        "unwired engine must produce an error reply, got: {:?}",
        msgs[0]
    );
}

/// Direct executor test: unwired engine returns `Err` from
/// `execute_workflow_start` (default reply text handled by common).
#[tokio::test]
async fn test_executor_unwired_returns_err() {
    let gw = make_gateway();
    let executor =
        crate::slash_permission::GatewaySlashExecutor::new(Arc::clone(&gw.session_manager), None);
    let err = executor.execute_workflow_start("sess-x", "deploy").await;
    assert!(err.is_err(), "unwired engine must return Err, got {err:?}");
}

/// Direct executor test: wired engine forwards to `WorkflowControl::start`.
#[tokio::test]
async fn test_executor_wired_forwards_to_control_start() {
    let control = Arc::new(FakeWorkflowControl::new(Ok(())));
    let gw = make_gateway();
    gw.session_manager
        .set_workflow_control(Arc::clone(&control) as Arc<dyn WorkflowControl>)
        .await;
    let executor =
        crate::slash_permission::GatewaySlashExecutor::new(Arc::clone(&gw.session_manager), None);
    let res = executor.execute_workflow_start("sess-y", "release").await;
    assert_eq!(res, Ok(()));
    assert_eq!(
        control.start_calls(),
        vec![("sess-y".to_owned(), "release".to_owned())]
    );
}
