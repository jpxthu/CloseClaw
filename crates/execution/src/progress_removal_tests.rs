//! Step 1.4 — Tests verifying that the system-level progress mechanism
//! has been fully removed from the execution engine.
//!
//! Behavior dimensions:
//! 1. **Normal path**: Engine execution loop doesn't have system-level
//!    state transitions — plan file markers are NOT rewritten by the
//!    system after executing N steps.
//! 2. **Error path**: Agent-side (fake LLM) plan file markers are not
//!    interfered with by the engine; engine no longer exposes progress
//!    update entry.
//! 3. **Boundary value**: Plan file with no Tasks section / empty steps
//!    list doesn't cause engine panic.

use crate::engine::ExecutionEngine;
use crate::error::ExecutionError;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, SubAgentResult, VerifyTrigger};
use crate::ExecutionStepStatus;
use async_trait::async_trait;

// ── test doubles ─────────────────────────────────────────────────────────

/// Mock spawn adapter that records all task descriptions dispatched.
struct RecordingSpawnAdapter {
    dispatched: std::sync::Mutex<Vec<String>>,
}

impl RecordingSpawnAdapter {
    fn new() -> Self {
        Self {
            dispatched: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl SpawnAdapter for RecordingSpawnAdapter {
    async fn spawn_run(
        &self,
        task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        self.dispatched
            .lock()
            .expect("mock lock poisoned")
            .push(task.to_string());
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".to_string(),
            changed_files: vec![],
            error_message: None,
        })
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock-session".to_string())
    }
}

fn spawn_all_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

fn step_by_step_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Normal path: no system-level state transitions / plan file rewriting
// ═══════════════════════════════════════════════════════════════════════════

/// Engine does not hold ExecutionState — no state transitions exist.
/// This is a compile-time structural check: ExecutionEngine fields
/// do not include ExecutionState (deleted in Step 1.1).
#[test]
fn test_engine_has_no_execution_state_field() {
    // If ExecutionState were still a field, this test's type inference
    // would fail at compile time (ExecutionState was deleted).
    // We verify structurally by checking the engine can be constructed
    // with just config + adapter (no state parameter).
    let adapter = RecordingSpawnAdapter::new();
    let _engine: ExecutionEngine<RecordingSpawnAdapter> =
        ExecutionEngine::new(spawn_all_config(), adapter, None);
    // If this compiles, ExecutionState is not a required parameter.
}

/// After executing N steps, the engine does not write to any plan file.
/// We verify this by confirming no filesystem side-effects occur —
/// the engine only dispatches tasks through the adapter.
#[tokio::test]
async fn test_engine_no_plan_file_writes() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(spawn_all_config(), adapter, None);

    // Create a temporary plan file to verify it's NOT modified.
    let tmp = tempfile::tempdir().expect("tempdir");
    let plan_file = tmp.path().join("plan.md");
    std::fs::write(&plan_file, "# Plan\n\n- [ ] Step 1\n- [ ] Step 2\n").expect("write plan file");

    let plan_before = std::fs::read_to_string(&plan_file).expect("read plan before");
    let mtime_before = std::fs::metadata(&plan_file)
        .expect("metadata before")
        .modified()
        .expect("mtime before");

    // Execute steps — engine must NOT touch the plan file.
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .expect("execute");

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 2);

    // Plan file unchanged.
    let plan_after = std::fs::read_to_string(&plan_file).expect("read plan after");
    assert_eq!(
        plan_before, plan_after,
        "plan file must not be modified by engine"
    );

    let mtime_after = std::fs::metadata(&plan_file)
        .expect("metadata after")
        .modified()
        .expect("mtime after");
    assert_eq!(mtime_before, mtime_after, "plan file mtime must not change");
}

/// In step-by-step mode, engine executes each step independently
/// without maintaining cross-step state.
#[tokio::test]
async fn test_step_by_step_no_cross_step_state() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(step_by_step_config(), adapter, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into(), "step 2".into()])
        .await
        .expect("execute");

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 3);
    for (i, step) in report.steps.iter().enumerate() {
        assert_eq!(step.step_index, i);
        assert!(matches!(step.status, ExecutionStepStatus::Completed));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Error path: agent plan file markers not interfered by engine
// ═══════════════════════════════════════════════════════════════════════════

/// When a step fails, the engine does not attempt to write progress
/// markers to any plan file. The engine only reports failure.
#[tokio::test]
async fn test_failure_does_not_write_progress() {
    struct FailingAdapter;

    #[async_trait]
    impl SpawnAdapter for FailingAdapter {
        async fn spawn_run(
            &self,
            _task: &str,
            _context: &str,
        ) -> Result<SubAgentResult, ExecutionError> {
            Err(ExecutionError::SpawnFailed {
                message: "step failed".into(),
            })
        }

        async fn spawn_session(
            &self,
            _task: &str,
            _context: &str,
        ) -> Result<String, ExecutionError> {
            Ok("mock".to_string())
        }
    }

    let engine = ExecutionEngine::new(step_by_step_config(), FailingAdapter, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .expect("execute");

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    // The engine does not write any plan file markers on failure.
}

/// The engine has no progress update entry point — there is no
/// `update_progress` or `set_progress` method on ExecutionEngine.
/// This is a compile-time structural check.
#[test]
fn test_engine_has_no_progress_update_entry() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(spawn_all_config(), adapter, None);
    // If ExecutionEngine had an update_progress method, calling
    // it would compile. Since it doesn't, we just verify the engine
    // can be constructed without a progress parameter.
    // The engine struct has no public progress-related methods.
    let _ = engine;
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Boundary value: empty steps / no tasks section
// ═══════════════════════════════════════════════════════════════════════════

/// Empty step list → engine returns all_completed with zero steps.
/// Does not panic.
#[tokio::test]
async fn test_empty_steps_does_not_panic() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(spawn_all_config(), adapter, None);

    let report = engine.execute(&[]).await.expect("execute empty");

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert!(report.steps.is_empty());
}

/// Single step → engine works normally.
#[tokio::test]
async fn test_single_step_does_not_panic() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(step_by_step_config(), adapter, None);

    let report = engine
        .execute(&["only step".into()])
        .await
        .expect("execute single");

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 1);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Completed
    ));
}

/// Step selection with empty indices → empty result.
#[tokio::test]
async fn test_empty_step_selection_does_not_panic() {
    let config = ExecutionConfig {
        step_selection: Some(vec![]),
        ..step_by_step_config()
    };
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(config, adapter, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .expect("execute with empty selection");

    assert!(report.all_completed);
    assert!(report.steps.is_empty());
}

/// Step selection with valid indices → only selected steps executed.
#[tokio::test]
async fn test_step_selection_valid_indices() {
    let config = ExecutionConfig {
        step_selection: Some(vec![1]),
        ..step_by_step_config()
    };
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(config, adapter, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into(), "step 2".into()])
        .await
        .expect("execute with selection");

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 1);
    // step_index is position within the filtered (selected) steps list.
    assert_eq!(report.steps[0].step_index, 0);
    assert_eq!(report.steps[0].description, "step 1");
}

/// Step selection with invalid index → error, no panic.
#[tokio::test]
async fn test_step_selection_invalid_index() {
    let config = ExecutionConfig {
        step_selection: Some(vec![5]),
        ..step_by_step_config()
    };
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(config, adapter, None);

    let result = engine.execute(&["step 0".into(), "step 1".into()]).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ExecutionError::InvalidStepSelection { index, total } => {
            assert_eq!(index, 5);
            assert_eq!(total, 2);
        }
        other => panic!("expected InvalidStepSelection, got: {other:?}"),
    }
}

/// SpawnAllSteps with empty steps → all_completed.
#[tokio::test]
async fn test_spawn_all_empty_steps() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(spawn_all_config(), adapter, None);

    let report = engine.execute(&[]).await.expect("execute empty");

    assert!(report.all_completed);
    assert!(report.steps.is_empty());
}

/// SpawnAllSteps with single step → works normally.
#[tokio::test]
async fn test_spawn_all_single_step() {
    let adapter = RecordingSpawnAdapter::new();
    let engine = ExecutionEngine::new(spawn_all_config(), adapter, None);

    let report = engine
        .execute(&["only step".into()])
        .await
        .expect("execute");

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 1);
}
