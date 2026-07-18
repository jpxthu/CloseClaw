//! Tests for ExecutionEngine step execution behavior.

use crate::engine::ExecutionEngine;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, NoopNotifier, PlanState};
use std::sync::{Arc, Mutex};

use crate::error::ExecutionError;

// ── Helpers ────────────────────────────────────────────────────────────────

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

fn default_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

/// Create an engine with a default plan state.
fn engine_with_default(
    adapter: MockAdapter,
) -> (ExecutionEngine<MockAdapter>, Arc<Mutex<PlanState>>) {
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state.clone(),
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    );
    (engine, plan_state)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_all_steps_succeed() {
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
    let (engine, _plan_state) = engine_with_default(adapter);
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
}

#[tokio::test]
async fn test_step_failure_stops_execution() {
    let adapter = MockAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
        Ok(SubAgentResult {
            step_index: 1,
            status: ExecutionStepStatus::Failed,
            summary: "fail".into(),
            changed_files: vec![],
            error_message: Some("broken".into()),
        }),
    ]);
    let (engine, _plan_state) = engine_with_default(adapter);
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(1));
}

#[tokio::test]
async fn test_empty_steps_succeed() {
    let adapter = MockAdapter::new(vec![]);
    let (engine, _plan_state) = engine_with_default(adapter);
    let report = engine.execute(&[]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert!(report.steps.is_empty());
}
