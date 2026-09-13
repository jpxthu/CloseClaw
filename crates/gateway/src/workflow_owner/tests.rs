//! Unit tests for `apply_resolve_action` (Step 1.3).

use closeclaw_common::ContentBlock;
use closeclaw_session::llm_session::ChatSession;
use closeclaw_session::workflow_handler::WorkflowHandler;
use closeclaw_session::workflow_recovery::DEFINITION_CHANGED_PAUSE_REASON;
use closeclaw_workflow::definition::{Step, Workflow};
use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

use super::Gateway;

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
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: closeclaw_workflow::run::PendingVerify {
            count: pending_verify,
            ..Default::default()
        },
        paused_reason: String::new(),
    }
}

fn make_session(
    phase: Phase,
    pending_verify: usize,
) -> closeclaw_session::llm_session::ConversationSession {
    make_session_with_reason(phase, pending_verify, "")
}

fn make_session_with_reason(
    phase: Phase,
    pending_verify: usize,
    paused_reason: &str,
) -> closeclaw_session::llm_session::ConversationSession {
    let mut cs = closeclaw_session::llm_session::ConversationSession::new(
        "test-sid".to_string(),
        "model".to_string(),
        std::path::PathBuf::from("/tmp"),
    );
    let mut run = make_test_run(phase, pending_verify);
    run.paused_reason = paused_reason.to_string();
    // Set both workflow_handler (for is_workflow_blocked) and workflow_run
    // (for paused_reason access via workflow_run()).
    let handler = WorkflowHandler::new(run.clone(), make_test_workflow());
    cs.set_workflow_handler(Some(handler));
    cs.set_workflow_run(Some(run));
    cs
}

/// Extract text content from a session message.
fn message_text(msg: &closeclaw_session::llm_session::SessionMessage) -> String {
    msg.content_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text(t) => Some(t.clone()),
            _ => None,
        })
        .next()
        .unwrap_or_default()
}

// ── Tests ──────────────────────────────────────────────────────────────

/// After resolve: goal message preserved, old verify cleaned, new verify
/// injected.
#[test]
fn test_resolve_preserves_goal_cleans_verify_injects_new() {
    let mut cs = make_session(Phase::Blocked, 3);
    // Inject goal and old verify messages.
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // Expect: goal + new verify = 2 workflow messages.
    assert_eq!(messages.len(), 2, "goal + new verify");
    assert_eq!(messages[0].role, "workflow");
    assert!(message_text(&messages[0]).starts_with("[workflow goal]"));
    assert_eq!(messages[1].role, "workflow");
    assert!(message_text(&messages[1]).starts_with("Verify Step"));
    // Old verify should be gone, new one present.
    let old_verify = "Verify Step 0 (Step 0):\nCheck output";
    let texts: Vec<String> = messages.iter().map(|m| message_text(m)).collect();
    assert_eq!(
        texts[1],
        closeclaw_workflow::definition::build_verify_message(
            &cs.workflow_handler().unwrap().definition().steps[0],
            true,
        )
    );
    // Old verify text must not appear.
    assert!(!texts.contains(&old_verify.to_string()));
}

/// Goal message content is consistent before and after resolve.
#[test]
fn test_resolve_goal_content_unchanged() {
    let mut cs = make_session(Phase::Blocked, 3);
    let goal_text = "[workflow goal] Step 0: Step 0\n\nDo first thing";
    cs.inject_workflow_message(goal_text);
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

    let goal_before = message_text(&cs.messages()[0]);
    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    let goal_after = message_text(&messages[0]);
    assert_eq!(goal_before, goal_after, "goal content must be unchanged");
}

/// pending_verify is zeroed and not overwritten by stale snapshot.
#[test]
fn test_resolve_pending_verify_zeroed_not_overwritten() {
    let mut cs = make_session(Phase::Blocked, 3);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    // Before resolve: pending_verify = 3.
    assert_eq!(cs.workflow_handler().unwrap().run().pending_verify.count, 3);
    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Blocked);

    Gateway::apply_resolve_action(&mut cs);

    // After resolve: pending_verify = 0, phase = Verifying.
    let handler = cs.workflow_handler().unwrap();
    assert_eq!(
        handler.run().pending_verify.count,
        0,
        "pending_verify must be zeroed"
    );
    assert_eq!(
        handler.run().phase,
        Phase::Verifying,
        "phase must be Verifying"
    );
}

/// After resolve with user messages interleaved, user messages are
/// preserved alongside goal and new verify.
#[test]
fn test_resolve_preserves_user_messages() {
    let mut cs = make_session(Phase::Blocked, 3);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");
    cs.append_transcript(
        "user",
        vec![ContentBlock::Text("please continue".to_string())],
    );

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // goal + user + new verify = 3.
    assert_eq!(messages.len(), 3, "goal + user + new verify");
    assert_eq!(messages[0].role, "workflow"); // goal
    assert!(message_text(&messages[0]).starts_with("[workflow goal]"));
    assert_eq!(messages[1].role, "user");
    assert_eq!(message_text(&messages[1]), "please continue");
    assert_eq!(messages[2].role, "workflow"); // new verify
    assert!(message_text(&messages[2]).starts_with("Verify Step"));
}

/// After resolve with no old verify messages, new verify is still injected.
#[test]
fn test_resolve_no_old_verify_still_injects_new() {
    let mut cs = make_session(Phase::Blocked, 0);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    Gateway::apply_resolve_action(&mut cs);

    let messages = cs.messages();
    // goal + new verify = 2.
    assert_eq!(messages.len(), 2, "goal + new verify");
    assert!(message_text(&messages[1]).starts_with("Verify Step"));
}

// ── Step 1.1: definition-changed pause resolve guard ───────────

/// When paused_reason matches DEFINITION_CHANGED_PAUSE_REASON,
/// apply_rejected_resolve_action injects an informational message
/// without changing phase/paused_reason/pending_verify.
#[test]
fn test_rejected_resolve_injects_message_without_changing_state() {
    let mut cs = make_session_with_reason(Phase::Blocked, 3, DEFINITION_CHANGED_PAUSE_REASON);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    cs.inject_workflow_message("Verify Step 0 (Step 0):\nCheck output");

    Gateway::apply_rejected_resolve_action(&mut cs);

    // Phase unchanged.
    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Blocked);
    // paused_reason unchanged.
    assert_eq!(
        cs.workflow_handler().unwrap().run().paused_reason,
        DEFINITION_CHANGED_PAUSE_REASON
    );
    // pending_verify unchanged.
    assert_eq!(cs.workflow_handler().unwrap().run().pending_verify.count, 3);
    // Rejection message injected.
    let msgs: Vec<String> = cs.messages().iter().map(|m| message_text(m)).collect();
    assert!(
        msgs.iter()
            .any(|m| m.contains("仅可回复「终止」结束工作流")),
        "rejection message must be present: {:?}",
        msgs
    );
    // Goal and old verify still present.
    assert!(msgs.iter().any(|m| m.starts_with("[workflow goal]")));
    assert!(msgs.iter().any(|m| m.starts_with("Verify Step")));
}

/// When paused_reason is empty (not a definition-changed pause),
/// apply_rejected_resolve_action should not be called; verify that
/// a normal resolve changes state as expected.
#[test]
fn test_normal_resolve_with_empty_paused_reason() {
    let mut cs = make_session_with_reason(Phase::Blocked, 3, "");
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    Gateway::apply_resolve_action(&mut cs);

    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Verifying);
    assert_eq!(cs.workflow_handler().unwrap().run().pending_verify.count, 0);
}

/// When paused_reason is a non-definition-changed value,
/// apply_rejected_resolve_action should not be called; verify that
/// a normal resolve changes state as expected.
#[test]
fn test_normal_resolve_with_other_paused_reason() {
    let mut cs = make_session_with_reason(Phase::Blocked, 3, "验收重试次数耗尽");
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    Gateway::apply_resolve_action(&mut cs);

    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Verifying);
    assert_eq!(cs.workflow_handler().unwrap().run().pending_verify.count, 0);
}

/// Terminate still works normally even with a definition-changed pause.
#[test]
fn test_terminate_works_with_definition_changed_pause() {
    let mut cs = make_session_with_reason(Phase::Blocked, 0, DEFINITION_CHANGED_PAUSE_REASON);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    Gateway::apply_terminate_action(&mut cs);

    // Workflow cleared.
    assert!(cs.workflow_handler().is_none());
    assert!(cs.workflow_run().is_none());
    // Messages cleared.
    let msgs: Vec<String> = cs.messages().iter().map(|m| message_text(m)).collect();
    assert!(
        msgs.is_empty(),
        "messages should be cleared after terminate"
    );
}

/// Empty paused_reason is a valid boundary: resolve proceeds normally.
#[test]
fn test_resolve_with_empty_paused_reason_is_not_rejected() {
    let mut cs = make_session_with_reason(Phase::Blocked, 0, "");
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");

    // Empty paused_reason should not trigger the guard.
    // apply_resolve_action is the correct path.
    Gateway::apply_resolve_action(&mut cs);

    assert_eq!(cs.workflow_handler().unwrap().run().phase, Phase::Verifying);
}

// ── Step 1.2: resolve_owner_action return value branches ───────

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{GatewayConfig, SessionManager};
use closeclaw_session::persistence::{PersistenceError, PersistenceService, SessionCheckpoint};

/// Minimal in-memory mock persistence for testing.
struct MockPersist(tokio::sync::Mutex<HashMap<String, SessionCheckpoint>>);

impl MockPersist {
    fn new() -> Self {
        Self(tokio::sync::Mutex::new(HashMap::new()))
    }
}

#[async_trait::async_trait]
impl PersistenceService for MockPersist {
    async fn save_checkpoint(&self, cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        self.0
            .lock()
            .await
            .insert(cp.session_id.clone(), cp.clone());
        Ok(())
    }
    async fn load_checkpoint(
        &self,
        sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(self.0.lock().await.get(sid).cloned())
    }
    async fn delete_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn purge_checkpoint(&self, _sid: &str) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn archive_checkpoint(&self, _cp: &SessionCheckpoint) -> Result<(), PersistenceError> {
        Ok(())
    }
    async fn restore_checkpoint(
        &self,
        _sid: &str,
    ) -> Result<Option<SessionCheckpoint>, PersistenceError> {
        Ok(None)
    }
    async fn list_active_sessions(&self) -> Result<Vec<String>, PersistenceError> {
        Ok(vec![])
    }
}

fn test_config() -> GatewayConfig {
    GatewayConfig::default()
}

/// Helper: build a Gateway + SessionManager wired to mock persistence,
/// with a conversation session for `sid` that has a workflow handler.
async fn setup_resolve_test(
    sid: &str,
    phase: Phase,
    paused_reason: &str,
) -> (crate::Gateway, Arc<SessionManager>) {
    let persist = Arc::new(MockPersist::new());
    // Save a checkpoint so get_sender_id can load sender_id.
    let mut cp = SessionCheckpoint::new(sid.to_string());
    cp.sender_id = Some("owner-123".to_string());
    persist.save_checkpoint(&cp).await.unwrap();

    let sm = Arc::new(SessionManager::new(
        &test_config(),
        Some(Arc::clone(&persist) as Arc<dyn PersistenceService>),
        None,
        Default::default(),
    ));
    // Register session in sessions map.
    sm.sessions.write().await.insert(
        sid.to_string(),
        crate::Session {
            id: sid.to_string(),
            agent_id: "test-agent".into(),
            channel: "mock".into(),
            created_at: 0,
            depth: 0,
        },
    );

    let cm = Arc::new(
        closeclaw_session::checkpoint_manager::CheckpointManager::new(
            Arc::clone(&persist) as Arc<dyn PersistenceService>
        ),
    );
    let gw = crate::Gateway::new(test_config(), Arc::clone(&sm)).with_checkpoint_manager(cm);

    // Insert a conversation session with a workflow handler.
    let mut cs = make_session_with_reason(phase, 3, paused_reason);
    cs.inject_workflow_message("[workflow goal] Step 0: Step 0\n\nDo first thing");
    sm.conversation_sessions
        .write()
        .await
        .insert(sid.to_string(), Arc::new(RwLock::new(cs)));

    (gw, sm)
}

/// Definition-changed pause → resolve returns Some("rejected_resolve").
#[tokio::test]
async fn test_resolve_owner_action_rejects_definition_changed_pause() {
    let (gw, _sm) = setup_resolve_test(
        "sid-reject",
        Phase::Blocked,
        DEFINITION_CHANGED_PAUSE_REASON,
    )
    .await;
    let result = gw
        .resolve_owner_action("sid-reject", Some("owner-123"), "恢复")
        .await;
    assert_eq!(result.as_deref(), Some("rejected_resolve"));
}

/// Normal blocked pause → resolve returns Some("resolve").
#[tokio::test]
async fn test_resolve_owner_action_allows_normal_blocked_pause() {
    let (gw, _sm) = setup_resolve_test("sid-resolve", Phase::Blocked, "Agent 主动阻塞").await;
    let result = gw
        .resolve_owner_action("sid-resolve", Some("owner-123"), "恢复")
        .await;
    assert_eq!(result.as_deref(), Some("resolve"));
}

/// Non-Blocked phase → resolve returns None.
#[tokio::test]
async fn test_resolve_owner_action_returns_none_when_not_blocked() {
    let (gw, _sm) = setup_resolve_test("sid-none", Phase::Verifying, "").await;
    let result = gw
        .resolve_owner_action("sid-none", Some("owner-123"), "恢复")
        .await;
    assert_eq!(result, None);
}

/// Empty paused_reason (boundary) → resolve returns Some("resolve").
#[tokio::test]
async fn test_resolve_owner_action_empty_paused_reason_not_rejected() {
    let (gw, _sm) = setup_resolve_test("sid-empty", Phase::Blocked, "").await;
    let result = gw
        .resolve_owner_action("sid-empty", Some("owner-123"), "恢复")
        .await;
    assert_eq!(result.as_deref(), Some("resolve"));
}

/// Terminate still works with definition-changed pause.
#[tokio::test]
async fn test_resolve_owner_action_terminate_works_with_definition_changed_pause() {
    let (gw, _sm) = setup_resolve_test(
        "sid-terminate",
        Phase::Blocked,
        DEFINITION_CHANGED_PAUSE_REASON,
    )
    .await;
    let result = gw
        .resolve_owner_action("sid-terminate", Some("owner-123"), "终止")
        .await;
    assert_eq!(result.as_deref(), Some("terminate"));
}

/// Non-owner sender → resolve returns None.
#[tokio::test]
async fn test_resolve_owner_action_rejects_non_owner() {
    let (gw, _sm) = setup_resolve_test("sid-nonowner", Phase::Blocked, "").await;
    let result = gw
        .resolve_owner_action("sid-nonowner", Some("random-user"), "恢复")
        .await;
    assert_eq!(result, None);
}

/// Unknown message content → resolve returns None.
#[tokio::test]
async fn test_resolve_owner_action_unknown_content_returns_none() {
    let (gw, _sm) = setup_resolve_test("sid-unknown", Phase::Blocked, "").await;
    let result = gw
        .resolve_owner_action("sid-unknown", Some("owner-123"), "something else")
        .await;
    assert_eq!(result, None);
}
