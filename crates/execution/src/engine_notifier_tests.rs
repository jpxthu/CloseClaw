//! Unit tests for `on_plan_completed` notification behavior (Step 1.5 — Gap 1).
//!
//! Verifies that `ExecutionEngine` calls `notifier.on_plan_completed()`
//! exactly when all steps complete successfully, for both step-by-step
//! and SpawnAllSteps execution modes.

use std::sync::{Arc, Mutex};

use crate::engine::ExecutionEngine;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, PlanState, PlanStateNotifier};

use crate::error::ExecutionError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct MockAdapter {
    results: std::sync::Mutex<Vec<Result<SubAgentResult, ExecutionError>>>,
}

impl MockAdapter {
    fn new(results: Vec<Result<SubAgentResult, ExecutionError>>) -> Self {
        Self {
            results: std::sync::Mutex::new(results),
        }
    }
}

#[async_trait]
impl SpawnAdapter for MockAdapter {
    async fn spawn_run(
        &self,
        _task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        let mut q = self.results.lock().expect("lock poisoned");
        q.remove(0)
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock".into())
    }
}

/// Mock notifier that records whether `on_plan_completed` was called.
struct RecordingNotifier {
    completed_called: Arc<Mutex<bool>>,
}

impl RecordingNotifier {
    fn new(completed_called: Arc<Mutex<bool>>) -> Self {
        Self { completed_called }
    }
}

#[async_trait]
impl PlanStateNotifier for RecordingNotifier {
    async fn on_progress_changed(&self, _progress_summary: &str) {}
    async fn on_plan_completed(&self) {
        *self.completed_called.lock().unwrap() = true;
    }
}

fn default_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
        max_retries: 3,
        retry_strategy: RetryStrategy::Fresh,
        verify_trigger: VerifyTrigger::NonTrivial,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_on_plan_completed_called_on_all_steps_success() {
    let completed = Arc::new(Mutex::new(false));
    let adapter = MockAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done 0".into(),
            changed_files: vec![],
            error_message: None,
        }),
        Ok(SubAgentResult {
            step_index: 1,
            status: ExecutionStepStatus::Completed,
            summary: "done 1".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(RecordingNotifier::new(completed.clone())),
        None,
    );
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert!(
        *completed.lock().unwrap(),
        "on_plan_completed should be called when all steps succeed"
    );
}

#[tokio::test]
async fn test_on_plan_completed_not_called_on_failure() {
    let completed = Arc::new(Mutex::new(false));
    let config = ExecutionConfig {
        max_retries: 0,
        ..default_config()
    };
    let adapter = MockAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "oops".into(),
        changed_files: vec![],
        error_message: Some("fail".into()),
    })]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        config,
        adapter,
        Arc::new(RecordingNotifier::new(completed.clone())),
        None,
    );
    let report = engine.execute(&["failing step".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert!(
        !*completed.lock().unwrap(),
        "on_plan_completed should NOT be called when steps fail"
    );
}

#[tokio::test]
async fn test_on_plan_completed_called_spawn_all_success() {
    let completed = Arc::new(Mutex::new(false));
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        ..default_config()
    };
    let adapter = MockAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "all done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        config,
        adapter,
        Arc::new(RecordingNotifier::new(completed.clone())),
        None,
    );
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert!(
        *completed.lock().unwrap(),
        "on_plan_completed should be called in SpawnAllSteps on success"
    );
}

#[tokio::test]
async fn test_on_plan_completed_not_called_spawn_all_failure() {
    let completed = Arc::new(Mutex::new(false));
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        max_retries: 0,
        ..default_config()
    };
    let adapter = MockAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "oops".into(),
        changed_files: vec![],
        error_message: Some("fail".into()),
    })]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        config,
        adapter,
        Arc::new(RecordingNotifier::new(completed.clone())),
        None,
    );
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert!(
        !*completed.lock().unwrap(),
        "on_plan_completed should NOT be called in SpawnAllSteps on failure"
    );
}
