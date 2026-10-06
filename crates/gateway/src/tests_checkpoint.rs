//! Tests for `SessionManager::save_checkpoint_after_compact` and the
//! workflow-state accessors it exposes (erased run storage, phase query,
//! post-compaction workflow context re-injection).

use crate::{GatewayConfig, SessionManager};
use closeclaw_common::{BootstrapMode, ModelSpec, SlashSessionQuery};
use closeclaw_session::llm_session::ConversationSession;
use closeclaw_session::persistence::ReasoningLevel;
use closeclaw_session::persistence::{PendingMessage, PersistenceService, SessionCheckpoint};
use closeclaw_workflow::run::{GoalHint, PendingVerify, Phase, WorkflowRun};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Mock persistence service ─────────────────────────────────────────────────

#[derive(Default)]
struct MockPersistence {
    checkpoints: RwLock<std::collections::HashMap<String, SessionCheckpoint>>,
}

#[async_trait::async_trait]
impl PersistenceService for MockPersistence {
    async fn save_checkpoint(
        &self,
        checkpoint: &SessionCheckpoint,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        self.checkpoints
            .write()
            .await
            .insert(checkpoint.session_id.clone(), checkpoint.clone());
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, closeclaw_session::persistence::PersistenceError> {
        Ok(self.checkpoints.read().await.get(session_id).cloned())
    }

    async fn delete_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_active_sessions(
        &self,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn archive_checkpoint(
        &self,
        _checkpoint: &SessionCheckpoint,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn restore_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<Option<SessionCheckpoint>, closeclaw_session::persistence::PersistenceError> {
        Ok(None)
    }

    async fn purge_checkpoint(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_archived_sessions(
        &self,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn invalidate_session(
        &self,
        _session_id: &str,
    ) -> Result<(), closeclaw_session::persistence::PersistenceError> {
        Ok(())
    }

    async fn list_idle_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: closeclaw_session::persistence::AgentRole,
        _idle_minutes: i64,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }

    async fn list_expired_archived_sessions_for_agent(
        &self,
        _agent_id: &str,
        _role: closeclaw_session::persistence::AgentRole,
        _purge_after_minutes: i64,
    ) -> Result<Vec<String>, closeclaw_session::persistence::PersistenceError> {
        Ok(Vec::new())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 10000,
        ..Default::default()
    }
}

async fn make_sm_with_storage(persistence: Arc<MockPersistence>) -> SessionManager {
    SessionManager::new(
        &make_config(),
        Some(persistence as Arc<dyn PersistenceService>),
        None,
        ReasoningLevel::default(),
    )
}

async fn register_conv_session(sm: &SessionManager, session_id: &str) {
    let mut cs = ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    );
    // Workflow state queries decode the stored Value through this port.
    cs.set_workflow_port(crate::test_support_workflow_port::test_port::test_port());
    let arc = Arc::new(RwLock::new(cs));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), arc);
}

async fn push_pending(sm: &SessionManager, session_id: &str, msg: PendingMessage) {
    let cs = sm
        .conversation_sessions
        .read()
        .await
        .get(session_id)
        .expect("session should exist")
        .clone();
    cs.write().await.push_pending(msg);
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Normal path: after compaction, outbound_pending is synced to checkpoint.
#[tokio::test]
async fn test_save_checkpoint_after_compact_syncs_outbound_pending() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;
    let session_id = "test-session-1";

    // Register ConversationSession with pending messages (simulating post-compaction)
    register_conv_session(&sm, session_id).await;
    push_pending(
        &sm,
        session_id,
        PendingMessage::new("boundary-1".into(), "summary after compact".into()),
    )
    .await;
    push_pending(
        &sm,
        session_id,
        PendingMessage::new("boundary-2".into(), "second boundary".into()),
    )
    .await;

    // Pre-populate checkpoint in storage (as if it existed before compaction)
    let mut old_cp = SessionCheckpoint::new(session_id.to_string());
    old_cp.touch();
    persistence
        .checkpoints
        .write()
        .await
        .insert(session_id.to_string(), old_cp);

    // Act
    sm.save_checkpoint_after_compact(session_id).await;

    // Assert: checkpoint's outbound_pending matches ConversationSession
    let saved = persistence
        .checkpoints
        .read()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should be saved");
    assert_eq!(
        saved.outbound_pending.len(),
        2,
        "checkpoint should have 2 pending messages after compaction"
    );
    assert_eq!(saved.outbound_pending[0].message_id, "boundary-1");
    assert_eq!(saved.outbound_pending[0].content, "summary after compact");
    assert_eq!(saved.outbound_pending[1].message_id, "boundary-2");
    assert_eq!(saved.outbound_pending[1].content, "second boundary");
}

/// Boundary: ConversationSession does not exist — method returns silently.
#[tokio::test]
async fn test_save_checkpoint_after_compact_no_session_returns_silently() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;

    // Pre-populate checkpoint in storage
    let cp = SessionCheckpoint::new("nonexistent-session".to_string());
    persistence
        .checkpoints
        .write()
        .await
        .insert("nonexistent-session".to_string(), cp);

    // Act: session_id has no ConversationSession — should not panic
    sm.save_checkpoint_after_compact("nonexistent-session")
        .await;

    // Assert: checkpoint still saved (outbound_pending unchanged from original)
    let saved = persistence
        .checkpoints
        .read()
        .await
        .get("nonexistent-session")
        .cloned()
        .expect("checkpoint should still be saved");
    assert!(
        saved.outbound_pending.is_empty(),
        "outbound_pending should remain empty when no ConversationSession exists"
    );
}

/// Boundary: storage not initialized — method returns silently.
#[tokio::test]
async fn test_save_checkpoint_after_compact_no_storage_returns_silently() {
    let sm = SessionManager::new(
        &make_config(),
        None, // no storage
        None,
        ReasoningLevel::default(),
    );

    // Act: should not panic when storage is None
    sm.save_checkpoint_after_compact("any-session").await;
}

// ── Workflow state access (erased run, phase query, compaction reinject) ──

/// Agent registry mock that resolves a fixed per-agent workspace, so the
/// workflow definition lookup hits the tempdir first (level 1).
struct WorkspaceRegistry {
    workspace: PathBuf,
}

#[async_trait::async_trait]
impl closeclaw_common::AgentLookup for WorkspaceRegistry {
    async fn get_agent_model(&self, _agent_id: &str) -> Option<ModelSpec> {
        None
    }
    async fn agent_exists(&self, _agent_id: &str) -> bool {
        true
    }
    async fn query_bootstrap_mode(&self, _agent_id: &str) -> Option<BootstrapMode> {
        None
    }
    async fn get_agent_workspace(&self, _agent_id: &str) -> Option<PathBuf> {
        Some(self.workspace.clone())
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentSkillsQuery for WorkspaceRegistry {
    fn get_agent_skills(&self, _agent_id: &str) -> Option<Vec<String>> {
        None
    }
}

#[async_trait::async_trait]
impl closeclaw_common::AgentToolsConfigQuery for WorkspaceRegistry {
    async fn get_agent_tools_config(
        &self,
        _agent_id: &str,
    ) -> Option<closeclaw_common::AgentToolsConfig> {
        None
    }
}

impl closeclaw_common::AgentRegistryQuery for WorkspaceRegistry {}

fn make_workflow_run(phase: Phase) -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test WF".to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: PendingVerify::default(),
        paused_reason: String::new(),
    }
}

/// Write `<dir>/workflows/Test WF/SKILL.md` with a one-step definition.
fn write_workflow_definition(dir: &std::path::Path) {
    let wf_dir = dir.join("workflows").join("Test WF");
    std::fs::create_dir_all(&wf_dir).unwrap();
    let yaml = concat!(
        "id: test-wf\n",
        "name: Test WF\n",
        "description: A test workflow\n",
        "steps:\n",
        "  - id: 0\n",
        "    name: Step 0\n",
        "    goal: Do first thing\n",
        "    verify:\n",
        "      - Check output\n",
        "    transitions:\n",
        "      - action: complete\n",
    );
    std::fs::write(
        wf_dir.join("SKILL.md"),
        format!("---\n{yaml}---\n\nBody.\n"),
    )
    .unwrap();
}

/// Store a checkpoint carrying `run` (optionally bound to an agent).
async fn seed_workflow_checkpoint(
    persistence: &Arc<MockPersistence>,
    session_id: &str,
    run: WorkflowRun,
    agent_id: Option<&str>,
) {
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.agent_id = agent_id.map(str::to_string);
    cp.workflow_run = Some(serde_json::to_value(run).unwrap());
    cp.touch();
    persistence
        .checkpoints
        .write()
        .await
        .insert(session_id.to_string(), cp);
}

fn count_workflow_contexts(appends: &[String]) -> usize {
    appends
        .iter()
        .filter(|a| a.starts_with("--- WORKFLOW ---"))
        .count()
}

/// Erased `set_workflow_run` stores the run on the session and persists it.
#[tokio::test]
async fn test_set_workflow_run_erased_sets_run_and_persists() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;
    let session_id = "wf-set-1";

    register_conv_session(&sm, session_id).await;
    let cs = sm.get_conversation_session(session_id).await.unwrap();
    cs.write().await.set_checkpoint_storage(persistence.clone());
    let mut cp = SessionCheckpoint::new(session_id.to_string());
    cp.touch();
    persistence
        .checkpoints
        .write()
        .await
        .insert(session_id.to_string(), cp);

    let run = make_workflow_run(Phase::Executing);
    let erased = serde_json::to_value(&run).unwrap();
    let result = SlashSessionQuery::set_workflow_run(&sm, session_id, Some(Box::new(erased))).await;
    assert!(
        result.is_ok(),
        "set_workflow_run should succeed: {result:?}"
    );

    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let stored = cs
        .read()
        .await
        .workflow_run_value()
        .expect("run should be stored on the session");
    assert_eq!(
        serde_json::from_value::<WorkflowRun>(stored).unwrap().phase,
        Phase::Executing
    );

    let saved = persistence
        .checkpoints
        .read()
        .await
        .get(session_id)
        .cloned()
        .expect("checkpoint should be persisted");
    let persisted = serde_json::from_value::<WorkflowRun>(saved.workflow_run.unwrap()).unwrap();
    assert_eq!(
        persisted.phase,
        Phase::Executing,
        "workflow run should reach the persisted checkpoint"
    );
}

/// Phase query reports the active phase and hides completed runs.
#[tokio::test]
async fn test_get_active_workflow_run_phase_query() {
    let sm = SessionManager::new(&make_config(), None, None, ReasoningLevel::default());
    let session_id = "wf-phase-1";
    register_conv_session(&sm, session_id).await;

    let cs = sm.get_conversation_session(session_id).await.unwrap();
    cs.write().await.set_workflow_run_value(Some(
        serde_json::to_value(make_workflow_run(Phase::Executing)).unwrap(),
    ));
    assert_eq!(
        sm.get_active_workflow_run_phase(session_id)
            .await
            .as_deref(),
        Some("Executing")
    );

    cs.write().await.set_workflow_run_value(Some(
        serde_json::to_value(make_workflow_run(Phase::Complete)).unwrap(),
    ));
    assert_eq!(
        sm.get_active_workflow_run_phase(session_id).await,
        None,
        "a completed run must not block a new workflow"
    );

    assert_eq!(
        sm.get_active_workflow_run_phase("no-such-session").await,
        None
    );
}

/// Missing workflow context after compaction → definition reloaded and
/// context re-injected exactly once.
#[tokio::test]
async fn test_reinject_workflow_context_after_compact_injects_missing_context() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;
    let session_id = "wf-reinject-1";
    register_conv_session(&sm, session_id).await;

    let workspace = tempfile::tempdir().unwrap();
    write_workflow_definition(workspace.path());
    sm.set_agent_registry(Arc::new(WorkspaceRegistry {
        workspace: workspace.path().to_path_buf(),
    }))
    .await;

    seed_workflow_checkpoint(
        &persistence,
        session_id,
        make_workflow_run(Phase::Executing),
        Some("agent-x"),
    )
    .await;

    sm.reinject_workflow_context_after_compact(session_id).await;

    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let appends = cs.read().await.system_appends();
    assert_eq!(
        count_workflow_contexts(&appends),
        1,
        "workflow context should be re-injected once"
    );
    assert!(
        appends.iter().any(|a| a.contains("Test WF")),
        "context should carry the reloaded definition: {appends:?}"
    );
}

/// Context already present → no duplicate injection.
#[tokio::test]
async fn test_reinject_workflow_context_after_compact_skips_existing_context() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;
    let session_id = "wf-reinject-2";
    register_conv_session(&sm, session_id).await;
    let cs = sm.get_conversation_session(session_id).await.unwrap();
    cs.write()
        .await
        .add_system_injection_append("--- WORKFLOW ---\nexisting".to_string());

    seed_workflow_checkpoint(
        &persistence,
        session_id,
        make_workflow_run(Phase::Executing),
        Some("agent-x"),
    )
    .await;

    sm.reinject_workflow_context_after_compact(session_id).await;

    let cs = sm.get_conversation_session(session_id).await.unwrap();
    let appends = cs.read().await.system_appends();
    assert_eq!(
        count_workflow_contexts(&appends),
        1,
        "existing context must not be duplicated"
    );
    assert!(
        appends.iter().any(|a| a == "--- WORKFLOW ---\nexisting"),
        "the pre-existing context should be untouched"
    );
}

/// Completed run / empty definition name → no injection.
#[tokio::test]
async fn test_reinject_workflow_context_after_compact_skips_inactive_runs() {
    let persistence = Arc::new(MockPersistence::default());
    let sm = make_sm_with_storage(persistence.clone()).await;

    let completed_id = "wf-reinject-complete";
    register_conv_session(&sm, completed_id).await;
    seed_workflow_checkpoint(
        &persistence,
        completed_id,
        make_workflow_run(Phase::Complete),
        Some("agent-x"),
    )
    .await;
    sm.reinject_workflow_context_after_compact(completed_id)
        .await;

    let empty_name_id = "wf-reinject-empty-name";
    register_conv_session(&sm, empty_name_id).await;
    let mut run = make_workflow_run(Phase::Executing);
    run.definition_name = String::new();
    seed_workflow_checkpoint(&persistence, empty_name_id, run, Some("agent-x")).await;
    sm.reinject_workflow_context_after_compact(empty_name_id)
        .await;

    for sid in [completed_id, empty_name_id] {
        let cs = sm.get_conversation_session(sid).await.unwrap();
        let appends = cs.read().await.system_appends();
        assert_eq!(
            count_workflow_contexts(&appends),
            0,
            "{sid} should not receive workflow context"
        );
    }
}
