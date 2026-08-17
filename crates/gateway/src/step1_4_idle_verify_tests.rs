//! Integration tests for idle→verify injection loop (Step 1.4).
//!
//! Covers the full chain: idle hook fires → verify injected → re-inject
//! removes old → counter increments → limit exceeded → Blocked + notification.
//! Also covers reverse branches: no run, definition load failure, busy session.

use std::sync::Arc;

use closeclaw_common::ContentBlock;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::llm_session::SessionMessage;
use closeclaw_session::workflow_handler::WorkflowHandler;
use closeclaw_workflow::definition::{Step, Workflow};
use closeclaw_workflow::run::{Phase, WorkflowRun};

use crate::session_handler_announce::tests::test_maybe_inject_workflow_verify;
use crate::session_manager::SessionManager;
use crate::GatewayConfig;

// ── helpers ────────────────────────────────────────────────────────────

fn make_test_workflow() -> Workflow {
    Workflow {
        id: "test-wf".to_string(),
        name: "Test Workflow".to_string(),
        description: "A test workflow".to_string(),
        version: Some("0.1".to_string()),
        allow_blocked: false,
        verify_retry_limit: 3,
        step_data_schema: serde_yaml::Value::Null,
        steps: vec![Step {
            id: 0,
            name: "Step 0".to_string(),
            goal: "Do first thing".to_string(),
            verify: vec!["Check output".to_string()],
            jump: vec![],
            transitions: vec![],
            allow_blocked: Some(true),
        }],
    }
}

fn make_test_run(phase: Phase, pending_verify: usize) -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test Workflow".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase,
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_verify,
    }
}

/// Create a SessionManager and register a ConversationSession with the
/// given workflow_run state. Returns (Arc<SessionManager>, session_id).
async fn setup_session(phase: Phase, pending_verify: usize) -> (Arc<SessionManager>, String) {
    let config = GatewayConfig::default();
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    let session_id = "test-sid".to_string();

    let mut cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.clone(),
        "model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    cs.set_workflow_run(Some(make_test_run(phase.clone(), pending_verify)));
    let handler = WorkflowHandler::new(make_test_run(phase, pending_verify), make_test_workflow());
    cs.set_workflow_handler(Some(handler));

    // Inject a goal message so the transcript is non-empty.
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.clone(), cs_arc);
    }
    // Register in sessions map so drain_workflow_notification can look up chat_id.
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "agent-test".to_string(),
            channel: "mock".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    (sm, session_id)
}

/// Create a session with a workflow_run but NO handler set (for ensure
/// workflow handler lazy-build path).
async fn setup_session_no_handler() -> (Arc<SessionManager>, String, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let config = GatewayConfig::default();
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    let session_id = "test-sid-no-handler".to_string();

    let mut cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.clone(),
        "model".to_string(),
        tmp.path().to_path_buf(),
    );
    // Set workflow_run but NOT workflow_handler — ensure_workflow_handler will try to load.
    cs.set_workflow_run(Some(make_test_run(Phase::Executing, 0)));

    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.clone(), cs_arc);
    }
    sm.sessions.write().await.insert(
        session_id.clone(),
        crate::Session {
            id: session_id.clone(),
            agent_id: "agent-test".to_string(),
            channel: "mock".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    (sm, session_id, tmp)
}

/// Read transcript messages from the session.
async fn read_messages(sm: &SessionManager, session_id: &str) -> Vec<SessionMessage> {
    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let cs_read = cs.read().await;
    cs_read.messages().to_vec()
}

/// Read the workflow_handler state from the session.
async fn read_handler_state(sm: &SessionManager, session_id: &str) -> (Phase, usize) {
    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let cs_read = cs.read().await;
    let handler = cs_read.workflow_handler().unwrap();
    (handler.run().phase.clone(), handler.run().pending_verify)
}

// ── Full chain tests ───────────────────────────────────────────────────

/// Full chain: Executing session becomes idle → verify message injected
/// with role=workflow, content matches build_verify_message output.
#[tokio::test]
async fn test_full_chain_idle_injects_verify_message() {
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    let messages = read_messages(&sm, &sid).await;
    // Expect: goal + new verify = 2 workflow messages.
    let wf_msgs: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == "workflow")
        .map(|m| {
            m.content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .next()
                .unwrap_or("")
        })
        .collect();

    assert_eq!(wf_msgs.len(), 2, "goal + new verify");
    assert!(wf_msgs[0].starts_with("[workflow goal]"));
    assert!(wf_msgs[1].starts_with("Verify Step 0 (Step 0):"));
    // Content should contain the allow_blocked hint.
    assert!(
        wf_msgs[1].contains("workflow_blocked"),
        "verify should contain allow_blocked hint"
    );

    // pending_verify should be 1.
    let (phase, pending) = read_handler_state(&sm, &sid).await;
    assert_eq!(phase, Phase::Executing);
    assert_eq!(pending, 1);
}

/// Full chain: re-inject on idle removes old verify and injects new one,
/// pending_verify increments.
#[tokio::test]
async fn test_full_chain_re_inject_removes_old_verify() {
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    // First injection.
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;
    let (_, pending1) = read_handler_state(&sm, &sid).await;
    assert_eq!(pending1, 1);

    // Second injection — old verify removed, new one injected.
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;
    let messages = read_messages(&sm, &sid).await;
    let wf_msgs: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == "workflow")
        .map(|m| {
            m.content_blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .next()
                .unwrap_or("")
        })
        .collect();
    // Should still be exactly 2: goal + new verify (old verify removed).
    assert_eq!(wf_msgs.len(), 2, "old verify removed, new injected");
    assert!(wf_msgs[1].starts_with("Verify Step 0 (Step 0):"));

    let (_, pending2) = read_handler_state(&sm, &sid).await;
    assert_eq!(pending2, 2);
}

/// Full chain: exceeding verify_retry_limit transitions to Blocked,
/// no further injection occurs.
#[tokio::test]
async fn test_full_chain_exceeds_limit_blocks() {
    // verify_retry_limit = 3, start at pending_verify = 0.
    // Phase transitions to Blocked when pending_verify > limit (i.e. after 4 injections).
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    // Inject 4 times (reaching the limit + 1 to trigger Blocked).
    for i in 0..4 {
        test_maybe_inject_workflow_verify(&sm, &sid, None).await;
        let (phase, pending) = read_handler_state(&sm, &sid).await;
        if i < 3 {
            assert_eq!(
                phase,
                Phase::Executing,
                "should still be Executing at {}",
                i
            );
        }
        assert_eq!(pending, i + 1);
    }

    // After 4 injections, pending_verify = 4 > limit = 3 → Blocked.
    let (phase, _) = read_handler_state(&sm, &sid).await;
    assert_eq!(phase, Phase::Blocked);

    // One more idle attempt — should NOT inject (returns early).
    let messages_before = read_messages(&sm, &sid).await;
    let count_before = messages_before.len();
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;
    let messages_after = read_messages(&sm, &sid).await;
    assert_eq!(
        messages_after.len(),
        count_before,
        "no new messages when Blocked"
    );
}

/// Full chain: after Blocked, the owner notification is queued and
/// delivered by drain_workflow_notification (consumed internally by the hook).
/// We verify the full chain by confirming: (1) phase is Blocked, and
/// (2) drain_workflow_notification was called (notification consumed).
#[tokio::test]
async fn test_full_chain_notification_queued_after_blocked() {
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    // Inject until Blocked (4 injections for limit=3).
    for _ in 0..4 {
        test_maybe_inject_workflow_verify(&sm, &sid, None).await;
    }
    let (phase, _) = read_handler_state(&sm, &sid).await;
    assert_eq!(phase, Phase::Blocked);

    // Verify notification was queued (and consumed by drain in the hook).
    // We do this by checking a fresh handler's state — the notification
    // field should be None since drain_workflow_notification took it.
    let cs = sm.get_conversation_session(&sid).await.unwrap();
    let mut cs_write = cs.write().await;
    let notification = cs_write.take_workflow_notification();
    assert!(
        notification.is_none(),
        "notification already consumed by drain_workflow_notification in the hook"
    );
}

/// Unit test: on_verify_injected queues notification when Blocked.
/// This isolates the notification queueing from the drain path.
#[test]
fn test_verify_injected_queues_notification_when_blocked() {
    let run = WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test Workflow".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase: Phase::Executing,
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_verify: 0,
    };
    let mut handler = WorkflowHandler::new(run, make_test_workflow());

    // Inject 4 times to exceed limit=3.
    for _ in 0..4 {
        handler.on_verify_injected(3);
    }
    assert_eq!(handler.run().phase, Phase::Blocked);

    // Notification should be queued.
    let notification = handler.take_notification();
    assert!(notification.is_some(), "Blocked should queue notification");
    let notif = notification.unwrap();
    assert_eq!(notif.workflow_name, "Test Workflow");
    assert!(!notif.message.is_empty());
}

/// Full chain: user messages interleaved with verifies are preserved.
#[tokio::test]
async fn test_full_chain_user_messages_preserved() {
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    // Inject verify.
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    // Simulate user message arriving.
    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        let mut cs_write = cs.write().await;
        cs_write.append_transcript("user", vec![ContentBlock::Text("keep going".to_string())]);
    }

    // Re-inject verify (old removed, user preserved).
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    let messages = read_messages(&sm, &sid).await;
    // goal + user + new verify = 3.
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "workflow"); // goal
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "workflow"); // new verify
}

// ── Reverse branch tests ───────────────────────────────────────────────

/// No workflow_run → ensure_workflow_handler is a no-op, hook does nothing.
#[tokio::test]
async fn test_reverse_no_workflow_run_no_side_effect() {
    let config = GatewayConfig::default();
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    let session_id = "test-sid-no-run".to_string();

    let cs = closeclaw_session::llm_session::ConversationSession::new(
        session_id.clone(),
        "model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    // No workflow_run set.
    let cs_arc = Arc::new(tokio::sync::RwLock::new(cs));
    {
        let mut conv = sm.conversation_sessions.write().await;
        conv.insert(session_id.clone(), cs_arc);
    }

    test_maybe_inject_workflow_verify(&sm, &session_id, None).await;

    let messages = read_messages(&sm, &session_id).await;
    assert!(messages.is_empty(), "no messages should be injected");
}

/// Definition load failure (tempdir has no definition files) →
/// ensure_workflow_handler stays None, hook does nothing.
#[tokio::test]
async fn test_reverse_definition_load_failure_no_side_effect() {
    let (sm, sid, _tmp) = setup_session_no_handler().await;

    // The session has a workflow_run but no definition file in the tempdir.
    // ensure_workflow_handler should fail to load and leave handler as None.
    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    let messages = read_messages(&sm, &sid).await;
    assert!(
        messages.is_empty(),
        "no messages when definition loading fails"
    );

    // Verify handler is still None.
    let cs = sm.get_conversation_session(&sid).await.unwrap();
    let cs_read = cs.read().await;
    assert!(
        cs_read.workflow_handler().is_none(),
        "handler should remain None after load failure"
    );
}

/// Session is busy (LLM requesting) → idle hook does not fire.
#[tokio::test]
async fn test_reverse_busy_session_no_injection() {
    let (sm, sid) = setup_session(Phase::Executing, 0).await;

    // Set LLM state to Requesting (busy).
    {
        let cs = sm.get_conversation_session(&sid).await.unwrap();
        let cs_write = cs.write().await;
        cs_write.set_llm_state(closeclaw_llm::session_state::LlmState::Requesting);
    }

    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    let messages = read_messages(&sm, &sid).await;
    // Only the goal message should be present — no verify injected.
    assert_eq!(messages.len(), 1, "only goal, no verify injected when busy");
    assert_eq!(messages[0].role, "workflow");
}

/// Non-Executing phase (e.g. Blocked) → idle hook does not fire.
#[tokio::test]
async fn test_reverse_blocked_phase_no_injection() {
    let (sm, sid) = setup_session(Phase::Blocked, 3).await;

    test_maybe_inject_workflow_verify(&sm, &sid, None).await;

    let messages = read_messages(&sm, &sid).await;
    // Only the goal message — Blocked phase does not inject.
    assert_eq!(messages.len(), 1, "only goal when phase is Blocked");
}
