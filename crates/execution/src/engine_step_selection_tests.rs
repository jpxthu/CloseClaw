//! Tests for step selection support (Step 1.3).
//!
//! Verifies that `ExecutionEngine::execute()` correctly filters steps
//! when `ExecutionConfig::step_selection` is `Some`.

use crate::engine::ExecutionEngine;
use crate::error::ExecutionError;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, NoopNotifier, PlanState};
use std::sync::{Arc, Mutex};

// ── Mock adapter ───────────────────────────────────────────────────────────

struct MockAdapter {
    results: Mutex<Vec<Result<SubAgentResult, ExecutionError>>>,
}

impl MockAdapter {
    fn new(results: Vec<Result<SubAgentResult, ExecutionError>>) -> Self {
        Self {
            results: Mutex::new(results),
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
        max_retries: 3,
        retry_strategy: RetryStrategy::Fresh,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_step_selection_none_executes_all() {
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
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    );
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 2);
}

#[tokio::test]
async fn test_step_selection_filters_single_step() {
    let adapter = MockAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let config = ExecutionConfig {
        step_selection: Some(vec![1]),
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into(), "step C".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].description, "step B");
}

#[tokio::test]
async fn test_step_selection_empty_executes_nothing() {
    let adapter = MockAdapter::new(vec![]);
    let config = ExecutionConfig {
        step_selection: Some(vec![]),
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert!(report.steps.is_empty());
}

#[tokio::test]
async fn test_step_selection_out_of_bounds_returns_error() {
    let adapter = MockAdapter::new(vec![]);
    let config = ExecutionConfig {
        step_selection: Some(vec![5]),
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let result = engine.execute(&["step A".into(), "step B".into()]).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        ExecutionError::InvalidStepSelection { index, total } => {
            assert_eq!(index, 5);
            assert_eq!(total, 2);
        }
        other => panic!("expected InvalidStepSelection, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_step_selection_multiple_indices() {
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
            summary: "done 2".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let config = ExecutionConfig {
        step_selection: Some(vec![0, 2]),
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into(), "step C".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.steps[0].description, "step A");
    assert_eq!(report.steps[1].description, "step C");
}

#[tokio::test]
async fn test_step_selection_preserves_failure_behavior() {
    let adapter = MockAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done A".into(),
            changed_files: vec![],
            error_message: None,
        }),
        // Step B (selected) fails via spawn error
        Err(ExecutionError::SpawnFailed {
            message: "step B failed".into(),
        }),
    ]);
    let config = ExecutionConfig {
        max_retries: 0,
        step_selection: Some(vec![0, 1]),
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into(), "step C".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(1));
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.steps[0].description, "step A");
    assert_eq!(report.steps[1].description, "step B");
}
