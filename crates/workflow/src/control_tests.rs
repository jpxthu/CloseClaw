//! Unit tests for [`WorkflowEngineControl`] over fake ports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use closeclaw_common::session_lookup::PendingMessage;
use closeclaw_common::{PlanState, SessionMode, SlashSessionQuery, WorkflowControl};

use crate::control::{OwnerPauseKind, WorkflowEngineControl, WorkflowSessionOps};
use crate::run::{PendingVerify, Phase, WorkflowRun};

// ── Fake SlashSessionQuery ──────────────────────────────────────────

#[derive(Default)]
struct FakeQueryState {
    active_phases: HashMap<String, Option<String>>,
    workdirs: HashMap<String, PathBuf>,
    set_calls: Vec<(String, Option<serde_json::Value>)>,
    injection_appends: Vec<(String, String)>,
    pending: Vec<(String, PendingMessage)>,
}

struct FakeQuery {
    state: Mutex<FakeQueryState>,
}

impl FakeQuery {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeQueryState::default()),
        })
    }

    fn set_active_phase(&self, session_id: &str, phase: Option<String>) {
        self.state
            .lock()
            .unwrap()
            .active_phases
            .insert(session_id.to_string(), phase);
    }

    fn set_workdir_for(&self, session_id: &str, path: PathBuf) {
        self.state
            .lock()
            .unwrap()
            .workdirs
            .insert(session_id.to_string(), path);
    }

    fn set_calls(&self) -> Vec<(String, Option<serde_json::Value>)> {
        self.state.lock().unwrap().set_calls.clone()
    }

    fn injection_appends(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().injection_appends.clone()
    }

    fn pending_messages(&self, session_id: &str) -> Vec<PendingMessage> {
        self.state
            .lock()
            .unwrap()
            .pending
            .iter()
            .filter(|(sid, _)| sid == session_id)
            .map(|(_, msg)| msg.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl SlashSessionQuery for FakeQuery {
    async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
        None
    }
    async fn set_plan_state(&self, _: &str, _: PlanState) {}
    async fn push_pending_message(
        &self,
        session_id: &str,
        msg: PendingMessage,
    ) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .pending
            .push((session_id.to_string(), msg));
        Ok(())
    }
    async fn trigger_manual_background(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    async fn set_workflow_run(
        &self,
        session_id: &str,
        run: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<(), String> {
        let value = run.map(|r| {
            *r.downcast::<serde_json::Value>()
                .expect("run handle downcasts to serde_json::Value")
        });
        self.state
            .lock()
            .unwrap()
            .set_calls
            .push((session_id.to_string(), value));
        Ok(())
    }
    async fn get_active_workflow_run_phase(&self, session_id: &str) -> Option<String> {
        self.state
            .lock()
            .unwrap()
            .active_phases
            .get(session_id)
            .cloned()
            .flatten()
    }
    async fn invalidate_static_cache(&self) {}
    async fn rebuild_system_prompt_for_session(&self, _: &str) {}
    async fn add_system_append(&self, _: &str, _: String) {}
    async fn add_system_injection_append(&self, session_id: &str, content: String) {
        self.state
            .lock()
            .unwrap()
            .injection_appends
            .push((session_id.to_string(), content));
    }
    async fn get_model(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_reasoning_level(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_verbosity_level(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_session_mode(&self, _: &str) -> Option<SessionMode> {
        None
    }
    async fn get_workdir(&self, session_id: &str) -> Option<PathBuf> {
        self.state.lock().unwrap().workdirs.get(session_id).cloned()
    }
    async fn get_system_appends(&self, _: &str) -> Vec<String> {
        vec![]
    }
    async fn set_workdir(&self, _: &str, _: PathBuf) {}
    async fn is_llm_busy(&self, _: &str) -> bool {
        false
    }
    async fn get_stats(&self, _: &str) -> Option<(usize, usize, usize, usize)> {
        None
    }
    async fn get_last_cache_anomaly(&self, _: &str) -> Option<String> {
        None
    }
    async fn get_active_child_count(&self, _: &str) -> usize {
        0
    }
}

// ── Fake WorkflowSessionOps ─────────────────────────────────────────

#[derive(Default)]
struct FakeOpsState {
    pause_kinds: HashMap<String, OwnerPauseKind>,
    runs: HashMap<String, Option<serde_json::Value>>,
    verify_removals: Vec<String>,
    message_removals: Vec<String>,
    context_removals: Vec<String>,
}

struct FakeOps {
    state: Mutex<FakeOpsState>,
}

impl FakeOps {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(FakeOpsState::default()),
        })
    }

    fn set_pause_kind(&self, session_id: &str, kind: OwnerPauseKind) {
        self.state
            .lock()
            .unwrap()
            .pause_kinds
            .insert(session_id.to_string(), kind);
    }

    fn set_run(&self, session_id: &str, run: Option<serde_json::Value>) {
        self.state
            .lock()
            .unwrap()
            .runs
            .insert(session_id.to_string(), run);
    }

    fn verify_removals(&self) -> Vec<String> {
        self.state.lock().unwrap().verify_removals.clone()
    }

    fn message_removals(&self) -> Vec<String> {
        self.state.lock().unwrap().message_removals.clone()
    }

    fn context_removals(&self) -> Vec<String> {
        self.state.lock().unwrap().context_removals.clone()
    }
}

#[async_trait::async_trait]
impl WorkflowSessionOps for FakeOps {
    async fn owner_pause_kind(&self, session_id: &str) -> OwnerPauseKind {
        self.state
            .lock()
            .unwrap()
            .pause_kinds
            .get(session_id)
            .copied()
            .unwrap_or(OwnerPauseKind::NotBlocked)
    }

    async fn read_workflow_run(&self, session_id: &str) -> Option<serde_json::Value> {
        self.state
            .lock()
            .unwrap()
            .runs
            .get(session_id)
            .cloned()
            .flatten()
    }

    async fn remove_workflow_verify_messages(&self, session_id: &str) {
        self.state
            .lock()
            .unwrap()
            .verify_removals
            .push(session_id.to_string());
    }

    async fn remove_workflow_messages(&self, session_id: &str) {
        self.state
            .lock()
            .unwrap()
            .message_removals
            .push(session_id.to_string());
    }

    async fn remove_workflow_context_append(&self, session_id: &str) {
        self.state
            .lock()
            .unwrap()
            .context_removals
            .push(session_id.to_string());
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

const SESSION: &str = "session-1";

/// Write a single-step workflow SKILL.md with the given version and goal.
fn write_workflow_file(dir: &std::path::Path, name: &str, version: &str, goal: &str) {
    let wf_dir = dir.join("workflows").join(name);
    std::fs::create_dir_all(&wf_dir).unwrap();
    let yaml = format!(
        concat!(
            "id: test-wf\n",
            "name: Test Workflow\n",
            "description: A test workflow\n",
            "version: \"{version}\"\n",
            "steps:\n",
            "  - id: 0\n",
            "    name: Step Zero\n",
            "    goal: {goal}\n",
            "    allow_blocked: true\n",
            "    verify:\n",
            "      - Check output\n",
            "    transitions:\n",
            "      - action: complete\n",
        ),
        version = version,
        goal = goal,
    );
    let content = format!("---\n{yaml}\n---\n\nBody.\n");
    std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
}

/// Build a control wired to fresh fakes; returns (control, query, ops).
fn make_control(
    global_workflows: Option<PathBuf>,
) -> (WorkflowEngineControl, Arc<FakeQuery>, Arc<FakeOps>) {
    let query = FakeQuery::new();
    let ops = FakeOps::new();
    let control = WorkflowEngineControl::new(query.clone(), ops.clone(), global_workflows);
    (control, query, ops)
}

/// Build a blocked run (verify-retry exhausted) for the given version.
fn blocked_run(definition_version: &str) -> serde_json::Value {
    let run = WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: "Test Workflow".to_string(),
        definition_version: definition_version.to_string(),
        current_step: 0,
        phase: Phase::Blocked,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: Default::default(),
        pending_verify: PendingVerify {
            count: 3,
            last_inject_time: "2026-01-01T00:00:00Z".to_string(),
            max_retry_limit: 3,
        },
        paused_reason: "验收重试次数耗尽".to_string(),
    };
    serde_json::to_value(&run).unwrap()
}

/// Decode the last persisted run snapshot for the session.
fn last_persisted_run(query: &FakeQuery) -> WorkflowRun {
    let calls = query.set_calls();
    let (_, value) = calls.last().expect("run must have been persisted");
    serde_json::from_value(value.clone().expect("run snapshot must be Some")).unwrap()
}

// ── start ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_start_success_full_chain() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_workflow_file(tmp.path(), "Test Workflow", "0.1", "Do the first thing");
    let (control, query, _ops) = make_control(Some(tmp.path().to_path_buf()));
    query.set_workdir_for(SESSION, tmp.path().to_path_buf());

    let result = control.start(SESSION, "Test Workflow").await;

    assert_eq!(result, Ok(()));
    // Run persisted with a Some snapshot.
    let calls = query.set_calls();
    assert_eq!(calls.len(), 1, "exactly one set_workflow_run call");
    assert_eq!(calls[0].0, SESSION);
    let run: WorkflowRun = serde_json::from_value(calls[0].1.clone().unwrap()).unwrap();
    assert_eq!(run.current_step, 0);
    assert_eq!(run.phase, Phase::Executing);
    assert_eq!(run.definition_name, "Test Workflow");
    // Workflow context appended via the system injection channel.
    let appends = query.injection_appends();
    assert_eq!(appends.len(), 1);
    assert_eq!(appends[0].0, SESSION);
    assert!(appends[0].1.starts_with("--- WORKFLOW ---"));
    assert!(appends[0].1.ends_with("--- WORKFLOW END ---"));
    // Step 0 goal message pushed as a workflow-role pending message.
    let pending = query.pending_messages(SESSION);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].role.as_deref(), Some("workflow"));
    assert!(pending[0].content.contains("[workflow goal] Step 0"));
    assert!(pending[0].content.contains("Do the first thing"));
}

#[tokio::test]
async fn test_start_rejects_when_active_workflow_exists() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_workflow_file(tmp.path(), "Test Workflow", "0.1", "Do the first thing");
    let (control, query, _ops) = make_control(Some(tmp.path().to_path_buf()));
    query.set_workdir_for(SESSION, tmp.path().to_path_buf());
    query.set_active_phase(SESSION, Some("Executing".to_string()));

    let result = control.start(SESSION, "Test Workflow").await;

    let err = result.expect_err("active workflow must reject start");
    assert!(err.contains("已有活跃的工作流运行"), "error: {err}");
    assert!(err.contains("Executing"), "error must carry phase: {err}");
    // No side effects.
    assert!(query.set_calls().is_empty());
    assert!(query.injection_appends().is_empty());
    assert!(query.pending_messages(SESSION).is_empty());
}

#[tokio::test]
async fn test_start_fails_when_definition_missing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (control, query, _ops) = make_control(Some(tmp.path().to_path_buf()));
    query.set_workdir_for(SESSION, tmp.path().to_path_buf());

    let result = control.start(SESSION, "NonExistent").await;

    let err = result.expect_err("missing definition must fail");
    assert!(
        err.contains("工作流 \"NonExistent\" 加载失败"),
        "error: {err}"
    );
    // No side effects.
    assert!(query.set_calls().is_empty());
    assert!(query.injection_appends().is_empty());
    assert!(query.pending_messages(SESSION).is_empty());
}

// ── on_owner_message ────────────────────────────────────────────────

#[tokio::test]
async fn test_owner_message_no_block_is_noop() {
    let (control, query, ops) = make_control(None);
    ops.set_pause_kind(SESSION, OwnerPauseKind::NotBlocked);

    let result = control.on_owner_message(SESSION).await;

    assert_eq!(result, Ok(()));
    assert!(ops.verify_removals().is_empty());
    assert!(ops.message_removals().is_empty());
    assert!(query.pending_messages(SESSION).is_empty());
    assert!(query.set_calls().is_empty());
}

#[tokio::test]
async fn test_owner_message_resolvable_full_chain() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_workflow_file(tmp.path(), "Test Workflow", "0.1", "Do the first thing");
    let (control, query, ops) = make_control(Some(tmp.path().to_path_buf()));
    query.set_workdir_for(SESSION, tmp.path().to_path_buf());
    ops.set_pause_kind(SESSION, OwnerPauseKind::Resolvable);
    ops.set_run(SESSION, Some(blocked_run("0.1")));

    let result = control.on_owner_message(SESSION).await;

    assert_eq!(result, Ok(()));
    // Verify residue cleaned; goal message preserved (no full removal).
    assert_eq!(ops.verify_removals(), vec![SESSION.to_string()]);
    assert!(ops.message_removals().is_empty());
    // Verify checklist injected as a workflow-role pending message.
    let pending = query.pending_messages(SESSION);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].role.as_deref(), Some("workflow"));
    assert!(
        pending[0].content.starts_with("Verify Step 0"),
        "checklist: {}",
        pending[0].content
    );
    // Run persisted: unblocked to verifying with counters reset.
    let run = last_persisted_run(&query);
    assert_eq!(run.phase, Phase::Verifying);
    assert_eq!(run.pending_verify.count, 0);
    assert!(run.paused_reason.is_empty());
}

#[tokio::test]
async fn test_owner_message_terminological_is_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_workflow_file(tmp.path(), "Test Workflow", "0.1", "Do the first thing");
    let (control, query, ops) = make_control(Some(tmp.path().to_path_buf()));
    ops.set_pause_kind(SESSION, OwnerPauseKind::Terminological);
    ops.set_run(SESSION, Some(blocked_run("0.1")));

    let result = control.on_owner_message(SESSION).await;

    assert_eq!(result, Ok(()));
    assert!(ops.verify_removals().is_empty());
    assert!(ops.message_removals().is_empty());
    assert!(query.pending_messages(SESSION).is_empty());
    assert!(query.set_calls().is_empty());
}

#[tokio::test]
async fn test_owner_message_definition_changed_replaces_goal() {
    let tmp = tempfile::TempDir::new().unwrap();
    // On-disk definition bumped to 0.2 with an updated goal text.
    write_workflow_file(tmp.path(), "Test Workflow", "0.2", "Updated goal text");
    let (control, query, ops) = make_control(Some(tmp.path().to_path_buf()));
    query.set_workdir_for(SESSION, tmp.path().to_path_buf());
    ops.set_pause_kind(SESSION, OwnerPauseKind::Resolvable);
    // Stored run still references version 0.1.
    ops.set_run(SESSION, Some(blocked_run("0.1")));

    let result = control.on_owner_message(SESSION).await;

    assert_eq!(result, Ok(()));
    // Full workflow message cleanup (old goal replaced), not verify-only.
    assert_eq!(ops.message_removals(), vec![SESSION.to_string()]);
    assert!(ops.verify_removals().is_empty());
    // Replacement goal + verify checklist injected.
    let pending = query.pending_messages(SESSION);
    assert_eq!(pending.len(), 2);
    assert!(pending[0].content.contains("[workflow goal] Step 0"));
    assert!(
        pending[0].content.contains("Updated goal text"),
        "goal must come from the latest definition: {}",
        pending[0].content
    );
    assert!(pending[1].content.starts_with("Verify Step 0"));
    // Persisted run records the latest definition version.
    let run = last_persisted_run(&query);
    assert_eq!(run.definition_version, "0.2");
    assert_eq!(run.phase, Phase::Verifying);
}

// ── terminate ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_terminate_with_active_run_cleans_up() {
    let (control, query, ops) = make_control(None);
    query.set_active_phase(SESSION, Some("Blocked".to_string()));

    let result = control.terminate(SESSION).await;

    assert_eq!(result, Ok(()));
    assert_eq!(ops.context_removals(), vec![SESSION.to_string()]);
    assert_eq!(ops.message_removals(), vec![SESSION.to_string()]);
    let calls = query.set_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, SESSION);
    assert!(calls[0].1.is_none(), "run must be cleared");
}

#[tokio::test]
async fn test_terminate_without_active_run_is_idempotent_noop() {
    let (control, query, ops) = make_control(None);

    let result = control.terminate(SESSION).await;

    assert_eq!(result, Ok(()));
    assert!(ops.context_removals().is_empty());
    assert!(ops.message_removals().is_empty());
    assert!(query.set_calls().is_empty());
}
