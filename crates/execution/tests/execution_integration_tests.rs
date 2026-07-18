//! Integration tests for the execution engine.
//!
//! Tests the full lifecycle: step dispatch, event emission,
//! and mode-specific behavior (SpawnAllSteps batching).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, NoopNotifier, PlanState};
use closeclaw_execution::error::ExecutionError;
use closeclaw_execution::event::ExecutionEvent;
use closeclaw_execution::spawn::SpawnAdapter;
use closeclaw_execution::types::{ExecutionConfig, ExecutionMode, SubAgentResult, VerifyTrigger};
use closeclaw_execution::ExecutionEngine;

// ---------------------------------------------------------------------------
// Mock adapters
// ---------------------------------------------------------------------------

/// Mock adapter that returns a fixed sequence of results.
struct SequenceMock {
    results: Mutex<Vec<Result<SubAgentResult, ExecutionError>>>,
}

impl SequenceMock {
    fn new(results: Vec<Result<SubAgentResult, ExecutionError>>) -> Self {
        Self {
            results: Mutex::new(results),
        }
    }
}

#[async_trait]
impl SpawnAdapter for SequenceMock {
    async fn spawn_run(
        &self,
        _task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        let mut queue = self.results.lock().expect("mock lock poisoned");
        queue.remove(0)
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock-session".into())
    }
}

/// Mock adapter that records calls and returns a fixed result.
#[derive(Clone)]
struct CallRecordingMock {
    result: SubAgentResult,
    calls: Arc<Mutex<Vec<String>>>,
}

impl CallRecordingMock {
    fn new(result: SubAgentResult) -> Self {
        Self {
            result,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl SpawnAdapter for CallRecordingMock {
    async fn spawn_run(
        &self,
        task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        self.calls
            .lock()
            .expect("mock lock poisoned")
            .push(task.to_string());
        Ok(self.result.clone())
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock-session".into())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn spawn_per_step_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

fn spawn_all_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        verify_trigger: VerifyTrigger::NonTrivial,
        step_selection: None,
    }
}

fn success_result(index: usize, summary: &str) -> SubAgentResult {
    SubAgentResult {
        step_index: index,
        status: ExecutionStepStatus::Completed,
        summary: summary.to_string(),
        changed_files: vec![],
        error_message: None,
    }
}

fn failed_result(index: usize, msg: &str) -> SubAgentResult {
    SubAgentResult {
        step_index: index,
        status: ExecutionStepStatus::Failed,
        summary: String::new(),
        changed_files: vec![],
        error_message: Some(msg.to_string()),
    }
}

fn new_engine_with_config(
    adapter: impl SpawnAdapter + 'static,
    config: ExecutionConfig,
) -> ExecutionEngine<impl SpawnAdapter> {
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None)
}

// ---------------------------------------------------------------------------
// Tests: Full 3-step success flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_3step_success() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "step 0 done")),
        Ok(success_result(1, "step 1 done")),
        Ok(success_result(2, "step 2 done")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine
        .execute(&["step A".into(), "step B".into(), "step C".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert_eq!(report.steps.len(), 3);

    for (i, step) in report.steps.iter().enumerate() {
        assert_eq!(step.step_index, i);
        assert!(matches!(step.status, ExecutionStepStatus::Completed));
        assert_eq!(step.attempts, 1);
    }

    // Verify step summaries
    assert_eq!(report.steps[0].summary, "step 0 done");
    assert_eq!(report.steps[1].summary, "step 1 done");
    assert_eq!(report.steps[2].summary, "step 2 done");
}

// ---------------------------------------------------------------------------
// Tests: Failure stops subsequent steps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_failure_stops_subsequent_steps() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "ok")),
        // Step 1 fails, step 2 should not be called
        Ok(failed_result(1, "fail")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine
        .execute(&["s0".into(), "s1".into(), "s2".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(1));
    // Only 2 results: step 0 completed, step 1 failed
    assert_eq!(report.steps.len(), 2);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Completed
    ));
    assert!(matches!(
        report.steps[1].status,
        ExecutionStepStatus::Failed
    ));
    assert_eq!(report.steps[1].attempts, 1);
}

// ---------------------------------------------------------------------------
// Tests: SpawnAllSteps — single spawn call for all steps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_all_steps_single_spawn() {
    let adapter = CallRecordingMock::new(success_result(0, "all done"));
    let calls = adapter.calls.clone();
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        spawn_all_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    );
    let report = engine
        .execute(&["s0".into(), "s1".into(), "s2".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 3);

    // Verify exactly one spawn_run call was made
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    // The combined task should contain all step descriptions
    assert!(calls[0].contains("s0"));
    assert!(calls[0].contains("s1"));
    assert!(calls[0].contains("s2"));
}

#[tokio::test]
async fn test_spawn_all_steps_failure_returns_failed_report() {
    let adapter = SequenceMock::new(vec![Err(ExecutionError::SpawnFailed {
        message: "batch fail".into(),
    })]);
    let engine = new_engine_with_config(adapter, spawn_all_config());
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    // SpawnAllSteps failure returns report with all steps failed
    assert!(!report.all_completed);
    assert_eq!(report.steps.len(), 2);
    for step in &report.steps {
        assert!(matches!(step.status, ExecutionStepStatus::Failed));
        assert_eq!(step.attempts, 1);
    }
}

// ---------------------------------------------------------------------------
// Tests: ExecutionEvent sequences
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_event_sequence_all_success() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "done")),
        Ok(success_result(1, "done")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    assert!(report.all_completed);

    // Expected events: StepStarted(0), StepCompleted(0),
    //                  StepStarted(1), StepCompleted(1), AllCompleted
    let events = &report.events;
    assert_eq!(events.len(), 5);

    assert_eq!(events[0], ExecutionEvent::StepStarted { step_index: 0 });
    assert_eq!(
        events[1],
        ExecutionEvent::StepCompleted {
            step_index: 0,
            summary: "done".into()
        }
    );
    assert_eq!(events[2], ExecutionEvent::StepStarted { step_index: 1 });
    assert_eq!(
        events[3],
        ExecutionEvent::StepCompleted {
            step_index: 1,
            summary: "done".into()
        }
    );
    assert_eq!(events[4], ExecutionEvent::AllCompleted);
}

#[tokio::test]
async fn test_event_sequence_failure_no_retry() {
    let adapter = SequenceMock::new(vec![
        Ok(failed_result(0, "oops")),
        // Step 1 should not be called
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    assert!(!report.all_completed);

    let events = &report.events;
    // Expected: StepStarted(0), StepFailed(0, ...)
    assert_eq!(events.len(), 2);

    assert_eq!(events[0], ExecutionEvent::StepStarted { step_index: 0 });
    assert!(matches!(
        &events[1],
        ExecutionEvent::StepFailed { step_index: 0, .. }
    ));
}

#[tokio::test]
async fn test_event_sequence_spawn_all_success() {
    let adapter = SequenceMock::new(vec![Ok(success_result(0, "batch done"))]);
    let engine = new_engine_with_config(adapter, spawn_all_config());
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    assert!(report.all_completed);

    let events = &report.events;
    // Expected: StepStarted(0), StepStarted(1),
    //           StepCompleted(0), StepCompleted(1), AllCompleted
    assert_eq!(events.len(), 5);

    assert_eq!(events[0], ExecutionEvent::StepStarted { step_index: 0 });
    assert_eq!(events[1], ExecutionEvent::StepStarted { step_index: 1 });
    assert!(matches!(
        &events[2],
        ExecutionEvent::StepCompleted { step_index: 0, .. }
    ));
    assert!(matches!(
        &events[3],
        ExecutionEvent::StepCompleted { step_index: 1, .. }
    ));
    assert_eq!(events[4], ExecutionEvent::AllCompleted);
}

// ---------------------------------------------------------------------------
// Tests: Empty steps, single step, edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_empty_steps() {
    let adapter = SequenceMock::new(vec![]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&[]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert!(report.steps.is_empty());
    // AllCompleted is emitted even for empty steps
    assert_eq!(report.events, vec![ExecutionEvent::AllCompleted]);
}

#[tokio::test]
async fn test_single_step_success() {
    let adapter = SequenceMock::new(vec![Ok(success_result(0, "only"))]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&["single".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].summary, "only");
    assert_eq!(report.events.len(), 3); // Started, Completed, AllCompleted
}

#[tokio::test]
async fn test_plan_state_updated_after_full_execution() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "done")),
        Ok(success_result(1, "done")),
    ]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state.clone(),
        spawn_per_step_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    );
    let _ = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    let state = plan_state.lock().unwrap();
    assert_eq!(state.execution_steps.len(), 2);
    assert!(matches!(
        state.execution_steps[0].status,
        ExecutionStepStatus::Completed
    ));
    assert!(matches!(
        state.execution_steps[1].status,
        ExecutionStepStatus::Completed
    ));
}

#[tokio::test]
async fn test_spawn_error_fails_immediately() {
    let adapter = SequenceMock::new(vec![Err(ExecutionError::SpawnFailed {
        message: "transient".into(),
    })]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&["step".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].attempts, 1);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Failed
    ));
}

#[tokio::test]
async fn test_failure_no_retry() {
    let adapter = SequenceMock::new(vec![Ok(failed_result(0, "fail"))]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine.execute(&["step".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.steps[0].attempts, 1);
}
