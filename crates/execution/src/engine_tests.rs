use crate::engine::{ExecutionEngine, StepResult};
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

/// Mock spawn adapter that records the context string passed to each spawn_run call.
struct ContextRecordingAdapter {
    results: Mutex<Vec<Result<SubAgentResult, ExecutionError>>>,
    /// Context strings passed to each spawn_run invocation, in order.
    contexts: Mutex<Vec<String>>,
}

impl ContextRecordingAdapter {
    fn new(results: Vec<Result<SubAgentResult, ExecutionError>>) -> Self {
        Self {
            results: Mutex::new(results),
            contexts: Mutex::new(Vec::new()),
        }
    }

    fn recorded_contexts(&self) -> Vec<String> {
        self.contexts.lock().expect("mock lock poisoned").clone()
    }
}

#[async_trait]
impl SpawnAdapter for ContextRecordingAdapter {
    async fn spawn_run(
        &self,
        _task: &str,
        context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        self.contexts
            .lock()
            .expect("mock lock poisoned")
            .push(context.to_string());
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
        None,
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
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
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
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
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
        None,
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
        None,
    );
    let _ = engine.execute(&["only step".into()]).await.unwrap();

    let state = plan_state.lock().unwrap();
    assert_eq!(state.execution_steps.len(), 1);
    assert!(matches!(
        state.execution_steps[0].status,
        ExecutionStepStatus::Completed
    ));
}

// ---------------------------------------------------------------------------
// retry_strategy tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_fresh_retry_passes_empty_context() {
    let adapter = ContextRecordingAdapter::new(vec![
        // First attempt fails
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Failed,
            summary: String::new(),
            changed_files: vec![],
            error_message: Some("flaky".into()),
        }),
        // Second attempt succeeds
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "fixed".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let config = ExecutionConfig {
        retry_strategy: RetryStrategy::Fresh,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine.execute(&["flaky step".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps[0].attempts, 2);

    // Fresh mode: all context strings must be empty
    let contexts = engine.adapter_ref().recorded_contexts();
    assert_eq!(contexts.len(), 2, "expected 2 spawn calls");
    assert!(
        contexts[0].is_empty(),
        "first attempt context should be empty"
    );
    assert!(
        contexts[1].is_empty(),
        "fresh retry context should be empty"
    );
}

#[tokio::test]
async fn test_continue_retry_passes_error_context() {
    let adapter = ContextRecordingAdapter::new(vec![
        // First attempt fails
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Failed,
            summary: String::new(),
            changed_files: vec![],
            error_message: Some("build error".into()),
        }),
        // Second attempt fails
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Failed,
            summary: String::new(),
            changed_files: vec![],
            error_message: Some("test failure".into()),
        }),
        // Third attempt succeeds
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let config = ExecutionConfig {
        retry_strategy: RetryStrategy::Continue,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine.execute(&["step".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(report.steps[0].attempts, 3);

    let contexts = engine.adapter_ref().recorded_contexts();
    assert_eq!(contexts.len(), 3, "expected 3 spawn calls");
    // First attempt: no error history, empty context
    assert!(contexts[0].is_empty(), "first attempt has no error history");
    // Second attempt: should carry first error
    assert!(
        contexts[1].contains("build error"),
        "second attempt should carry first error, got: {}",
        contexts[1]
    );
    // Third attempt: should carry both previous errors
    assert!(
        contexts[2].contains("build error") && contexts[2].contains("test failure"),
        "third attempt should carry both errors, got: {}",
        contexts[2]
    );
}

#[tokio::test]
async fn test_continue_retry_spawn_all_passes_error_context() {
    let adapter = ContextRecordingAdapter::new(vec![
        // First attempt fails
        Err(ExecutionError::SpawnFailed {
            message: "network timeout".into(),
        }),
        // Second attempt succeeds
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        retry_strategy: RetryStrategy::Continue,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    let contexts = engine.adapter_ref().recorded_contexts();
    assert_eq!(contexts.len(), 2, "expected 2 spawn calls");
    assert!(contexts[0].is_empty(), "first attempt has no error history");
    assert!(
        contexts[1].contains("network timeout"),
        "continue retry should carry error context, got: {}",
        contexts[1]
    );
}

#[tokio::test]
async fn test_fresh_retry_spawn_all_passes_empty_context() {
    let adapter = ContextRecordingAdapter::new(vec![
        Err(ExecutionError::SpawnFailed {
            message: "boom".into(),
        }),
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
    ]);
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        retry_strategy: RetryStrategy::Fresh,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);
    let report = engine
        .execute(&["step A".into(), "step B".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    let contexts = engine.adapter_ref().recorded_contexts();
    assert_eq!(contexts.len(), 2, "expected 2 spawn calls");
    assert!(contexts[0].is_empty());
    assert!(
        contexts[1].is_empty(),
        "fresh retry context should be empty"
    );
}

// ---------------------------------------------------------------------------
// verify_trigger auto-construction tests (G3)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_new_engine_uses_config_verify_trigger() {
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(
        plan_state,
        default_config(),
        adapter,
        Arc::new(NoopNotifier),
        None,
    );

    // new() should auto-construct a HookRunner with config.verify_trigger
    let hook_runner = engine
        .hook_runner_ref()
        .expect("hook_runner should be Some after new()");

    // NonTrivial: hook runs when step has changed_files
    let step_no_files = StepResult {
        step_index: 0,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: String::new(),
        changed_files: vec![],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    };
    assert!(!hook_runner.should_run(&step_no_files));

    let step_with_files = StepResult {
        step_index: 0,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: String::new(),
        changed_files: vec!["file.rs".into()],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    };
    assert!(hook_runner.should_run(&step_with_files));
}

#[tokio::test]
async fn test_new_engine_verify_trigger_always() {
    let config = ExecutionConfig {
        verify_trigger: VerifyTrigger::Always,
        ..default_config()
    };
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);

    let hook_runner = engine
        .hook_runner_ref()
        .expect("hook_runner should be Some");

    let step_no_files = StepResult {
        step_index: 0,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: String::new(),
        changed_files: vec![],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    };
    assert!(hook_runner.should_run(&step_no_files));
}

#[tokio::test]
async fn test_new_engine_verify_trigger_never() {
    let config = ExecutionConfig {
        verify_trigger: VerifyTrigger::Never,
        ..default_config()
    };
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::new(plan_state, config, adapter, Arc::new(NoopNotifier), None);

    let hook_runner = engine
        .hook_runner_ref()
        .expect("hook_runner should be Some");

    let step_with_files = StepResult {
        step_index: 0,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: String::new(),
        changed_files: vec!["file.rs".into()],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    };
    assert!(!hook_runner.should_run(&step_with_files));
}

#[tokio::test]
async fn test_with_hook_runner_overrides_config_trigger() {
    let config = ExecutionConfig {
        verify_trigger: VerifyTrigger::Never,
        ..default_config()
    };
    let adapter = MockSpawnAdapter::new(vec![]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));

    // Build a hook runner with Always trigger (overrides config's Never)
    let custom_runner = HookRunner::new(VerifyTrigger::Always);

    let engine = ExecutionEngine::with_hook_runner(
        plan_state,
        config,
        adapter,
        Arc::new(NoopNotifier),
        custom_runner,
        None,
    );

    let hook_runner = engine
        .hook_runner_ref()
        .expect("hook_runner should be Some");

    // Should use Always (from custom runner), not Never (from config)
    let step_no_files = StepResult {
        step_index: 0,
        description: "test".into(),
        status: ExecutionStepStatus::Completed,
        summary: String::new(),
        changed_files: vec![],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    };
    assert!(hook_runner.should_run(&step_no_files));
}

// ---------------------------------------------------------------------------
// Hook Block signal propagation tests (G4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_hook_block_sets_hook_blocked_on_step_result() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(BlockingHook)]);
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert_eq!(
        report.steps[0].hook_blocked.as_deref(),
        Some("blocked by hook")
    );
}

#[tokio::test]
async fn test_hook_block_sets_hook_blocked_on_execution_report() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(BlockingHook)]);
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.hook_blocked);
}

#[tokio::test]
async fn test_hook_block_stops_subsequent_steps() {
    let adapter = MockSpawnAdapter::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "done".into(),
            changed_files: vec![],
            error_message: None,
        }),
        // Step 1 should never be dispatched
    ]);
    let engine = engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(BlockingHook)]);
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    assert!(report.hook_blocked);
    assert_eq!(report.steps.len(), 1);
    assert_eq!(
        report.steps[0].hook_blocked.as_deref(),
        Some("blocked by hook")
    );
    assert!(!report.steps[0].error_message.is_some());
    // Step 1 never executed
    assert!(!report.steps.iter().any(|s| s.step_index == 1));
}

#[tokio::test]
async fn test_continue_hook_does_not_set_hook_blocked() {
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
        vec![Box::new(RecordingHook::new(Arc::new(Mutex::new(0))))],
    );
    let report = engine.execute(&["step 0".into()]).await.unwrap();

    assert!(report.all_completed);
    assert!(!report.hook_blocked);
    assert!(report.steps[0].hook_blocked.is_none());
}

#[tokio::test]
async fn test_no_hook_runner_does_not_set_hook_blocked() {
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
    assert!(!report.hook_blocked);
    assert!(report.steps[0].hook_blocked.is_none());
}

// ---------------------------------------------------------------------------
// SpawnAllSteps Hook execution tests (G8)
// ---------------------------------------------------------------------------

fn spawn_all_engine_with_hooks(
    adapter: MockSpawnAdapter,
    trigger: VerifyTrigger,
    hooks: Vec<Box<dyn StepHook>>,
) -> ExecutionEngine<MockSpawnAdapter> {
    let mut runner = HookRunner::new(trigger);
    for hook in hooks {
        runner.register(hook);
    }
    let config = ExecutionConfig {
        mode: ExecutionMode::SpawnAllSteps,
        ..default_config()
    };
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    ExecutionEngine::with_hook_runner(
        plan_state,
        config,
        adapter,
        Arc::new(NoopNotifier),
        runner,
        None,
    )
}

#[tokio::test]
async fn test_spawn_all_hook_triggered() {
    let count = Arc::new(Mutex::new(0usize));
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine = spawn_all_engine_with_hooks(
        adapter,
        VerifyTrigger::Always,
        vec![Box::new(RecordingHook::new(count.clone()))],
    );
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    // Hooks run for step 0 + step 1 = 2 calls
    assert_eq!(*count.lock().unwrap(), 2);
    assert!(report.events.iter().any(|e| matches!(
        e,
        crate::event::ExecutionEvent::HookExecuted { step_index: 0 }
    )));
    assert!(report.events.iter().any(|e| matches!(
        e,
        crate::event::ExecutionEvent::HookExecuted { step_index: 1 }
    )));
}

#[tokio::test]
async fn test_spawn_all_hook_block_propagates() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let engine =
        spawn_all_engine_with_hooks(adapter, VerifyTrigger::Always, vec![Box::new(BlockingHook)]);
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    // All steps still marked completed (SpawnAllSteps dispatches as one)
    assert!(report.all_completed);
    assert!(report.hook_blocked);
    assert_eq!(
        report.steps[0].hook_blocked.as_deref(),
        Some("blocked by hook")
    );
    assert_eq!(
        report.steps[1].hook_blocked.as_deref(),
        Some("blocked by hook")
    );
    assert!(report.events.iter().any(|e| matches!(
        e,
        crate::event::ExecutionEvent::HookFailed {
            step_index: 0,
            error_message,
        } if error_message == "blocked by hook"
    )));
    assert!(report.events.iter().any(|e| matches!(
        e,
        crate::event::ExecutionEvent::HookFailed {
            step_index: 1,
            error_message,
        } if error_message == "blocked by hook"
    )));
}

#[tokio::test]
async fn test_spawn_all_failure_skips_hooks() {
    let count = Arc::new(Mutex::new(0usize));
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Failed,
        summary: "oops".into(),
        changed_files: vec![],
        error_message: Some("fail".into()),
    })]);
    let engine = spawn_all_engine_with_hooks(
        adapter,
        VerifyTrigger::Always,
        vec![Box::new(RecordingHook::new(count.clone()))],
    );
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(!report.all_completed);
    // No hooks should run when step 0 fails
    assert_eq!(*count.lock().unwrap(), 0);
}
