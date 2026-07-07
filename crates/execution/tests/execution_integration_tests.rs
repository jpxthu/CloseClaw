//! Integration tests for the execution engine.
//!
//! Tests the full lifecycle: step dispatch, retry, event emission,
//! and mode-specific behavior (SpawnAllSteps batching).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, NoopNotifier, PlanState};
use closeclaw_execution::error::ExecutionError;
use closeclaw_execution::event::ExecutionEvent;
use closeclaw_execution::spawn::SpawnAdapter;
use closeclaw_execution::types::{
    ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger,
};
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

fn spawn_per_step_config(max_retries: u32) -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
        max_retries,
        retry_strategy: RetryStrategy::Fresh,
        verify_trigger: VerifyTrigger::NonTrivial,
    }
}

fn spawn_all_config(max_retries: u32) -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        max_retries,
        retry_strategy: RetryStrategy::Fresh,
        verify_trigger: VerifyTrigger::NonTrivial,
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
    ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier))
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
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
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
// Tests: Middle step failure + retry success
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_middle_failure_then_retry() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "ok")),
        // Step 1 fails first
        Ok(failed_result(1, "flaky")),
        // Step 1 succeeds on retry
        Ok(success_result(1, "recovered")),
        Ok(success_result(2, "done")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
    let report = engine
        .execute(&["s0".into(), "s1".into(), "s2".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 3);
    // Step 1 had 2 attempts
    assert_eq!(report.steps[1].attempts, 2);
    assert_eq!(report.steps[1].summary, "recovered");
}

// ---------------------------------------------------------------------------
// Tests: Retry exhaustion — stops after max retries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_retry_exhaustion_stops() {
    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "ok")),
        // Step 1 fails, retries exhausted
        Ok(failed_result(1, "fail 1")),
        Ok(failed_result(1, "fail 2")),
        Ok(failed_result(1, "fail 3")),
        // Step 2 should never be called
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(2));
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
    assert_eq!(report.steps[1].attempts, 3);
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
        spawn_all_config(3),
        adapter,
        Arc::new(NoopNotifier),
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
async fn test_spawn_all_steps_failure_triggers_retry() {
    let adapter = SequenceMock::new(vec![
        Err(ExecutionError::SpawnFailed {
            message: "batch fail".into(),
        }),
        Ok(success_result(0, "batch ok")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_all_config(3));
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 2);
    // Both steps should be completed
    for step in &report.steps {
        assert!(matches!(step.status, ExecutionStepStatus::Completed));
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
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
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
async fn test_event_sequence_failure_with_retry() {
    let adapter = SequenceMock::new(vec![
        // Step 0 fails, retries, then succeeds
        Ok(failed_result(0, "oops")),
        Ok(success_result(0, "fixed")),
        // Step 1 succeeds
        Ok(success_result(1, "ok")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
    let report = engine.execute(&["s0".into(), "s1".into()]).await.unwrap();

    assert!(report.all_completed);

    let events = &report.events;
    // Expected: StepStarted(0), RetryTriggered(0,2),
    //           StepStarted(0), StepCompleted(0),
    //           StepStarted(1), StepCompleted(1), AllCompleted
    // StepStarted(0) on first attempt, RetryTriggered(0,2),
    // StepStarted(0) on retry, StepCompleted(0),
    // StepStarted(1), StepCompleted(1), AllCompleted
    assert_eq!(events.len(), 6);

    assert_eq!(events[0], ExecutionEvent::StepStarted { step_index: 0 });
    assert_eq!(
        events[1],
        ExecutionEvent::RetryTriggered {
            step_index: 0,
            attempt: 2
        }
    );
    // StepStarted(0) is NOT emitted on retry (only on first attempt)
    assert_eq!(
        events[2],
        ExecutionEvent::StepCompleted {
            step_index: 0,
            summary: "fixed".into()
        }
    );
    assert_eq!(events[3], ExecutionEvent::StepStarted { step_index: 1 });
    assert_eq!(
        events[4],
        ExecutionEvent::StepCompleted {
            step_index: 1,
            summary: "ok".into()
        }
    );
    assert_eq!(events[5], ExecutionEvent::AllCompleted);
}

#[tokio::test]
async fn test_event_sequence_retry_exhaustion() {
    let adapter = SequenceMock::new(vec![
        Ok(failed_result(0, "fail 1")),
        Ok(failed_result(0, "fail 2")),
        Ok(failed_result(0, "fail 3")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(2));
    let report = engine.execute(&["s0".into()]).await.unwrap();

    assert!(!report.all_completed);

    let events = &report.events;
    // Expected: StepStarted(0), RetryTriggered(0,2),
    //           StepStarted(0), RetryTriggered(0,3),
    //           StepStarted(0), StepFailed(0, ...)
    // StepStarted(0), RetryTriggered(0,2), RetryTriggered(0,3), StepFailed(0)
    assert_eq!(events.len(), 4);

    assert_eq!(events[0], ExecutionEvent::StepStarted { step_index: 0 });
    assert_eq!(
        events[1],
        ExecutionEvent::RetryTriggered {
            step_index: 0,
            attempt: 2
        }
    );
    assert_eq!(
        events[2],
        ExecutionEvent::RetryTriggered {
            step_index: 0,
            attempt: 3
        }
    );
    assert!(matches!(
        &events[3],
        ExecutionEvent::StepFailed { step_index: 0, .. }
    ));
}

#[tokio::test]
async fn test_event_sequence_spawn_all_success() {
    let adapter = SequenceMock::new(vec![Ok(success_result(0, "batch done"))]);
    let engine = new_engine_with_config(adapter, spawn_all_config(3));
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
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
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
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
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
        spawn_per_step_config(3),
        adapter,
        Arc::new(NoopNotifier),
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
async fn test_spawn_error_retries_then_succeeds() {
    let adapter = SequenceMock::new(vec![
        Err(ExecutionError::SpawnFailed {
            message: "transient".into(),
        }),
        Ok(success_result(0, "recovered")),
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(3));
    let report = engine.execute(&["step".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps[0].attempts, 2);
    assert_eq!(report.steps[0].summary, "recovered");
}

#[tokio::test]
async fn test_max_retries_zero_no_retry() {
    let adapter = SequenceMock::new(vec![Ok(failed_result(0, "fail"))]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config(0));
    let report = engine.execute(&["step".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.steps[0].attempts, 1);
}
