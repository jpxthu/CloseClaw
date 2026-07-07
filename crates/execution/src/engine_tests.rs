use crate::engine::ExecutionEngine;
use crate::error::ExecutionError;
use crate::hook::{HookResult, HookRunner, StepHook};
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, NoopNotifier, PlanState};
use std::sync::{Arc, Mutex};

/// Mock spawn adapter that returns a configurable sequence of results.
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
        _task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        let mut queue = self.results.lock().expect("mock lock poisoned");
        queue.remove(0)
    }

    async fn spawn_session(&self, _task: &str, _context: &str) -> Result<String, ExecutionError> {
        Ok("mock-session".to_string())
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

fn new_engine(adapter: MockSpawnAdapter) -> ExecutionEngine<MockSpawnAdapter> {
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
    )
}

#[tokio::test]
async fn test_empty_steps_all_completed() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let engine = new_engine(adapter);
    let report = engine.execute(&[]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert!(report.steps.is_empty());
}

#[tokio::test]
async fn test_all_steps_succeed() {
    let adapter = MockSpawnAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done 0".to_string(),
            changed_files: vec!["a.rs".into()],
            error_message: None,
        }),
        Ok(SubAgentResult {
            step_index: 1,
            status: ExecutionStepStatus::Completed,
            summary: "done 1".to_string(),
            changed_files: vec!["b.rs".into()],
            error_message: None,
        }),
        Ok(SubAgentResult {
            step_index: 2,
            status: ExecutionStepStatus::Completed,
            summary: "done 2".to_string(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let engine = new_engine(adapter);
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
}

#[tokio::test]
async fn test_single_step_failure_then_retry_success() {
    let adapter = MockSpawnAdapter::new(vec![
        // First attempt fails
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Failed,
            summary: "oops".to_string(),
            changed_files: vec![],
            error_message: Some("flaky".into()),
        }),
        // Second attempt succeeds
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "fixed".to_string(),
            changed_files: vec!["fixed.rs".into()],
            error_message: None,
        }),
    ]);
    let engine = new_engine(adapter);
    let report = engine.execute(&["flaky step".into()]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.failed_step.is_none());
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].attempts, 2);
    assert_eq!(report.steps[0].summary, "fixed");
}

#[tokio::test]
async fn test_spawn_error_exhausts_retries() {
    let config = ExecutionConfig {
        max_retries: 2,
        ..default_config()
    };
    let adapter = MockSpawnAdapter::new(vec![
        Err(ExecutionError::SpawnFailed {
            message: "boom".into(),
        }),
        Err(ExecutionError::SpawnFailed {
            message: "boom 2".into(),
        }),
        Err(ExecutionError::SpawnFailed {
            message: "boom 3".into(),
        }),
    ]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier));
    let report = engine.execute(&["doomed step".into()]).await.unwrap();

    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].attempts, 3);
    let actual = report.steps[0].error_message.as_deref();
    assert!(
        actual == Some("spawn failed: boom 3"),
        "expected 'spawn failed: boom 3', got: {actual:?}"
    );
}

#[tokio::test]
async fn test_failure_stops_subsequent_steps() {
    let config = ExecutionConfig {
        max_retries: 0,
        ..default_config()
    };
    let adapter = MockSpawnAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "ok".into(),
            changed_files: vec![],
            error_message: None,
        }),
        // Step 1 fails, step 2 should not execute
        Err(ExecutionError::SpawnFailed {
            message: "step1 fail".into(),
        }),
    ]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier));
    let report = engine
        .execute(&["step 0".into(), "step 1".into(), "step 2".into()])
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
}

// ---------------------------------------------------------------------------
// Hook integration tests
// ---------------------------------------------------------------------------

/// Mock hook that records calls.
struct RecordingHook {
    call_count: Arc<Mutex<usize>>,
}

impl RecordingHook {
    fn new(call_count: Arc<Mutex<usize>>) -> Self {
        Self { call_count }
    }
}

#[async_trait]
impl StepHook for RecordingHook {
    async fn execute(
        &self,
        _step: &crate::engine::StepResult,
    ) -> Result<HookResult, crate::hook::HookError> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        Ok(HookResult::Continue)
    }
}

/// Mock hook that blocks.
struct BlockingHook;

#[async_trait]
impl StepHook for BlockingHook {
    async fn execute(
        &self,
        _step: &crate::engine::StepResult,
    ) -> Result<HookResult, crate::hook::HookError> {
        Ok(HookResult::Block("blocked by hook".into()))
    }
}

/// Mock hook that fails.
struct ErrorHook;

#[async_trait]
impl StepHook for ErrorHook {
    async fn execute(
        &self,
        _step: &crate::engine::StepResult,
    ) -> Result<HookResult, crate::hook::HookError> {
        Err(crate::hook::HookError::CustomFailed {
            message: "hook error".into(),
        })
    }
}

fn engine_with_hooks(
    adapter: MockSpawnAdapter,
    trigger: VerifyTrigger,
    hooks: Vec<Box<dyn StepHook>>,
) -> ExecutionEngine<MockSpawnAdapter> {
    let mut runner = HookRunner::new(trigger);
    for hook in hooks {
        runner.register(hook);
    }
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    ExecutionEngine::with_hook_runner(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        runner,
    )
}

#[tokio::test]
async fn test_hook_never_skips_hooks() {
    let count = Arc::new(Mutex::new(0usize));
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec!["file.rs".into()],
        error_message: None,
    })]);
    let engine = engine_with_hooks(
        adapter,
        VerifyTrigger::Never,
        vec![Box::new(RecordingHook::new(count.clone()))],
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(*count.lock().unwrap(), 0);
    assert!(!report
        .events
        .iter()
        .any(|e| matches!(e, crate::event::ExecutionEvent::HookExecuted { .. })));
}

#[tokio::test]
async fn test_hook_always_triggers() {
    let count = Arc::new(Mutex::new(0usize));
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = engine_with_hooks(
        adapter,
        VerifyTrigger::Always,
        vec![Box::new(RecordingHook::new(count.clone()))],
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(*count.lock().unwrap(), 1);
    assert!(report.events.iter().any(|e| matches!(
        e,
        crate::event::ExecutionEvent::HookExecuted { step_index: 0 }
    )));
}

#[tokio::test]
async fn test_hook_block_records_event() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(BlockingHook)]);
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    // Step still completes even though hook blocked
    assert!(report.all_completed);
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, crate::event::ExecutionEvent::HookFailed {
        step_index: 0,
        error_message,
    } if error_message == "blocked by hook")));
}

#[tokio::test]
async fn test_hook_failure_does_not_block_step() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(ErrorHook)]);
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert!(report.steps[0].status == ExecutionStepStatus::Completed);
}

#[tokio::test]
async fn test_plan_state_updated_after_execution() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state.clone(),
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
    );
    let _ = engine.execute(&["only step".into()]).await.unwrap();

    let state = plan_state.lock().unwrap();
    assert_eq!(state.execution_steps.len(), 1);
    assert!(matches!(
        state.execution_steps[0].status,
        ExecutionStepStatus::Completed
    ));
}
