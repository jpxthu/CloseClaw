// ---------------------------------------------------------------------------
// Permission integration tests (G6)
// ---------------------------------------------------------------------------

use crate::engine::ExecutionEngine;
use crate::error::ExecutionError;
use crate::event::ExecutionEvent;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::{
    ExecutionPermissionCheck, ExecutionStepStatus, NoopNotifier, PermissionDenied, PlanState,
};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock implementations
// ---------------------------------------------------------------------------

struct MockSpawnAdapter {
    results: Mutex<Vec<Result<SubAgentResult, ExecutionError>>>,
}

impl MockSpawnAdapter {
    fn new(results: Vec<Result<SubAgentResult, ExecutionError>>) -> Self {
        Self {
            results: Mutex::new(results),
        }
    }
}

#[async_trait]
impl SpawnAdapter for MockSpawnAdapter {
    async fn spawn_run(
        &self,
        _description: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        self.results.lock().unwrap().remove(0)
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock-session".into())
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

fn new_engine(adapter: MockSpawnAdapter) -> ExecutionEngine<MockSpawnAdapter> {
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    )
}

// ---------------------------------------------------------------------------
// Permission mock implementations
// ---------------------------------------------------------------------------

/// Mock permission checker that always allows.
struct AllowPermission;

#[async_trait]
impl ExecutionPermissionCheck for AllowPermission {
    async fn check_execution(&self, _step: &str) -> Result<(), PermissionDenied> {
        Ok(())
    }
}

/// Mock permission checker that always denies.
struct DenyPermission;

#[async_trait]
impl ExecutionPermissionCheck for DenyPermission {
    async fn check_execution(&self, _step: &str) -> Result<(), PermissionDenied> {
        Err(PermissionDenied::new("not allowed"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_permission_pass_allows_execution() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        Some(Arc::new(AllowPermission)),
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert!(report.steps[0].error_message.is_none());
}

#[tokio::test]
async fn test_permission_deny_marks_step_failed() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        Some(Arc::new(DenyPermission)),
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    assert_eq!(report.steps.len(), 1);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Failed
    ));
    assert!(report.steps[0]
        .error_message
        .as_deref()
        .unwrap()
        .contains("permission denied"));
    // Only 1 attempt — no retry on permission denial
    assert_eq!(report.steps[0].attempts, 1);
}

#[tokio::test]
async fn test_permission_deny_stops_subsequent_steps() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        Some(Arc::new(DenyPermission)),
    );
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    assert_eq!(report.steps.len(), 1);
}

#[tokio::test]
async fn test_no_permission_checker_allows_execution() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = new_engine(adapter);
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
}

#[tokio::test]
async fn test_permission_deny_records_step_failed_event() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        Some(Arc::new(DenyPermission)),
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.events.iter().any(|e| matches!(
        e,
        ExecutionEvent::StepFailed {
            step_index: 0,
            error_message,
        } if error_message.contains("permission denied")
    )));
}

#[tokio::test]
async fn test_permission_deny_spawn_all_marks_all_steps_failed() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        config,
        adapter,
        Arc::new(NoopNotifier),
        Some(Arc::new(DenyPermission)),
    );
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    assert_eq!(report.steps.len(), 2);
    for step in &report.steps {
        assert!(matches!(step.status, ExecutionStepStatus::Failed));
        assert!(step
            .error_message
            .as_deref()
            .unwrap()
            .contains("permission denied"));
    }
    // No retries attempted
    for step in &report.steps {
        assert_eq!(step.attempts, 0);
    }
}
