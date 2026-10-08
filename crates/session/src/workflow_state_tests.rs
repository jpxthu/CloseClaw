//! Tests for the workflow state accessors exposed to cross-module consumers.
//!
//! Covers the seams gateway uses instead of depending on the workflow crate:
//! phase queries over the stored `Value`, workflow-context detection,
//! checkpoint definition-name queries and post-compaction context rebuild.

use crate::llm_session::ConversationSession;
use crate::persistence::SessionCheckpoint;
use crate::workflow_port::real_engine_port::test_port;
use crate::workflow_port::WorkflowRunDecodeError;
use crate::workflow_recovery::rebuild_workflow_context_append;
use closeclaw_workflow::run::{GoalHint, Phase, WorkflowRun};

fn make_run(phase: Phase) -> WorkflowRun {
    make_run_named("Test WF", phase)
}

fn make_run_named(definition_name: &str, phase: Phase) -> WorkflowRun {
    WorkflowRun {
        workflow_id: "test-wf".to_string(),
        definition_name: definition_name.to_string(),
        definition_version: "0.1".to_string(),
        current_step: 0,
        phase,
        current_step_entered_at: "2026-01-01T00:00:00Z".to_string(),
        step_history: vec![],
        step_data: serde_yaml::Value::Null,
        pending_goal_hint: GoalHint::default(),
        pending_verify: closeclaw_workflow::run::PendingVerify::default(),
        paused_reason: String::new(),
    }
}

fn make_session() -> ConversationSession {
    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        std::path::PathBuf::from("unused-workdir"),
    );
    session.set_workflow_port(test_port());
    session
}

/// Store a run on a checkpoint in the persisted `Value` form.
fn seed_run(cp: &mut SessionCheckpoint, run: WorkflowRun) {
    cp.workflow_run = Some(serde_json::to_value(run).unwrap());
}

/// Write `<dir>/workflows/<name>/SKILL.md` with a one-step definition.
fn write_workflow_definition(dir: &std::path::Path, name: &str) {
    let wf_dir = dir.join("workflows").join(name);
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
    let content = format!("---\n{yaml}---\n\nBody.\n");
    std::fs::write(wf_dir.join("SKILL.md"), content).unwrap();
}

// ── active_workflow_run_phase: port requirement ───────────────────────────

#[test]
fn test_active_workflow_run_phase_without_port_is_ok_none() {
    // No port injected → the stored Value cannot be decoded here; the query
    // degrades to "no active run" instead of naming workflow types.
    let mut session = ConversationSession::new(
        "sid".to_string(),
        "model".to_string(),
        std::path::PathBuf::from("unused-workdir"),
    );
    session.set_workflow_run(Some(make_run(Phase::Executing)));
    assert_eq!(session.active_workflow_run_phase(), Ok(None));
}

// ── active_workflow_run_phase ─────────────────────────────────────────────

#[test]
fn test_active_workflow_run_phase_no_run_is_none() {
    assert_eq!(make_session().active_workflow_run_phase(), Ok(None));
}

#[test]
fn test_active_workflow_run_phase_active_is_debug_string() {
    let mut session = make_session();
    session.set_workflow_run(Some(make_run(Phase::Executing)));
    assert_eq!(
        session.active_workflow_run_phase(),
        Ok(Some("Executing".to_string()))
    );
}

#[test]
fn test_active_workflow_run_phase_complete_is_none() {
    let mut session = make_session();
    session.set_workflow_run(Some(make_run(Phase::Complete)));
    assert_eq!(
        session.active_workflow_run_phase(),
        Ok(None),
        "completed runs must not count as active (one-run-per-session rule)"
    );
}

/// A malformed stored value surfaces as `Err` for the caller to log —
/// the session must not emit the gateway-owned warn itself.
#[test]
fn test_active_workflow_run_phase_undecodable_run_is_err() {
    let mut session = make_session();
    session.set_workflow_run_value(Some(serde_json::json!({ "not": "a run" })));
    assert_eq!(
        session.active_workflow_run_phase(),
        Err(WorkflowRunDecodeError)
    );
}

// ── has_workflow_context ──────────────────────────────────────────────────

#[test]
fn test_has_workflow_context_absent_by_default() {
    assert!(!make_session().has_workflow_context());
}

#[test]
fn test_has_workflow_context_detects_injected_append() {
    let mut session = make_session();
    session.add_system_injection_append("--- WORKFLOW ---\nctx".to_string());
    assert!(session.has_workflow_context());
}

// ── checkpoint active_workflow_definition_name ────────────────────────────

#[test]
fn test_active_workflow_definition_name_no_run() {
    let cp = SessionCheckpoint::new("sid".to_string());
    assert_eq!(
        cp.active_workflow_definition_name(test_port().as_ref()),
        Ok(None)
    );
}

#[test]
fn test_active_workflow_definition_name_complete_run_is_none() {
    let mut cp = SessionCheckpoint::new("sid".to_string());
    seed_run(&mut cp, make_run(Phase::Complete));
    assert_eq!(
        cp.active_workflow_definition_name(test_port().as_ref()),
        Ok(None)
    );
}

#[test]
fn test_active_workflow_definition_name_active_run() {
    let mut cp = SessionCheckpoint::new("sid".to_string());
    seed_run(&mut cp, make_run(Phase::Verifying));
    assert_eq!(
        cp.active_workflow_definition_name(test_port().as_ref()),
        Ok(Some("Test WF".to_string()))
    );
}

#[test]
fn test_active_workflow_definition_name_empty_name_still_reported() {
    let mut cp = SessionCheckpoint::new("sid".to_string());
    seed_run(&mut cp, make_run_named("", Phase::Executing));
    assert_eq!(
        cp.active_workflow_definition_name(test_port().as_ref()),
        Ok(Some("".to_string())),
        "an empty name must be reported so callers can warn"
    );
}

/// A malformed stored value surfaces as `Err` for the caller to log —
/// the session must not emit the gateway-owned warn itself.
#[test]
fn test_active_workflow_definition_name_undecodable_run_is_err() {
    let mut cp = SessionCheckpoint::new("sid".to_string());
    cp.workflow_run = Some(serde_json::json!({ "not": "a run" }));
    assert_eq!(
        cp.active_workflow_definition_name(test_port().as_ref()),
        Err(WorkflowRunDecodeError)
    );
}

// ── rebuild_workflow_context_append ───────────────────────────────────────

#[test]
fn test_rebuild_workflow_context_append_from_agent_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    write_workflow_definition(tmp.path(), "Test WF");

    let ctx = rebuild_workflow_context_append(test_port().as_ref(), "Test WF", Some(tmp.path()))
        .expect("definition exists in the agent workspace");
    assert!(
        ctx.starts_with("--- WORKFLOW ---"),
        "context must carry the workflow marker: {ctx}"
    );
    assert!(ctx.contains("Test WF"), "context should name the workflow");
}

#[test]
fn test_rebuild_workflow_context_append_missing_definition_is_err() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        rebuild_workflow_context_append(
            test_port().as_ref(),
            "__closeclaw_missing_wf__",
            Some(tmp.path()),
        )
        .is_err(),
        "unknown definition must surface the loader error to the caller"
    );
}
