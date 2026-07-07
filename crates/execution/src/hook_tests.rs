use crate::engine::StepResult;
use crate::error::ExecutionError;
use crate::hook::{
    CustomHook, HookError, HookResult, HookRunner, NotifyHook, StepHook, VerificationHook,
};
use crate::spawn::SpawnAdapter;
use crate::types::{SubAgentResult, VerifyTrigger};
use async_trait::async_trait;
use closeclaw_common::ExecutionStepStatus;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Mock implementations
// ---------------------------------------------------------------------------

/// Mock spawn adapter for verification hook tests.
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
        Ok("mock-session".into())
    }
}

/// Mock step hook that records calls and optionally blocks.
struct MockHook {
    call_count: Arc<Mutex<usize>>,
    block_on: Option<usize>,
}

impl MockHook {
    fn new(call_count: Arc<Mutex<usize>>, block_on: Option<usize>) -> Self {
        Self {
            call_count,
            block_on,
        }
    }
}

#[async_trait]
impl StepHook for MockHook {
    async fn execute(&self, _step: &StepResult) -> Result<HookResult, HookError> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        let current = *count;
        if self.block_on == Some(current) {
            Ok(HookResult::Block("mock block".into()))
        } else {
            Ok(HookResult::Continue)
        }
    }
}

/// Mock hook that always fails.
struct FailingHook;

#[async_trait]
impl StepHook for FailingHook {
    async fn execute(&self, _step: &StepResult) -> Result<HookResult, HookError> {
        Err(HookError::CustomFailed {
            message: "intentional failure".into(),
        })
    }
}

fn completed_step() -> StepResult {
    StepResult {
        step_index: 0,
        description: "test step".into(),
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec!["file.rs".into()],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    }
}

fn no_changes_step() -> StepResult {
    StepResult {
        step_index: 1,
        description: "no changes step".into(),
        status: ExecutionStepStatus::Completed,
        summary: "done".into(),
        changed_files: vec![],
        error_message: None,
        attempts: 1,
        hook_blocked: None,
    }
}

// ---------------------------------------------------------------------------
// HookRunner tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_hook_runner_always_triggers() {
    let count = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(MockHook::new(count.clone(), None)));

    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Continue);
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test]
async fn test_hook_runner_never_skips() {
    let count = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::Never);
    runner.register(Box::new(MockHook::new(count.clone(), None)));

    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Continue);
    assert_eq!(*count.lock().unwrap(), 0);
}

#[tokio::test]
async fn test_hook_runner_nontrivial_skips_empty_files() {
    let count = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::NonTrivial);
    runner.register(Box::new(MockHook::new(count.clone(), None)));

    let result = runner.run_hooks(&no_changes_step()).await;
    assert_eq!(result, HookResult::Continue);
    assert_eq!(*count.lock().unwrap(), 0);
}

#[tokio::test]
async fn test_hook_runner_nontrivial_triggers_with_files() {
    let count = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::NonTrivial);
    runner.register(Box::new(MockHook::new(count.clone(), None)));

    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Continue);
    assert_eq!(*count.lock().unwrap(), 1);
}

#[tokio::test]
async fn test_hook_runner_block_stops_subsequent() {
    let count1 = Arc::new(Mutex::new(0usize));
    let count2 = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(MockHook::new(count1.clone(), Some(1))));
    runner.register(Box::new(MockHook::new(count2.clone(), None)));

    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Block("mock block".into()));
    assert_eq!(*count1.lock().unwrap(), 1);
    assert_eq!(*count2.lock().unwrap(), 0); // second hook never called
}

#[tokio::test]
async fn test_hook_runner_failure_does_not_block() {
    let count = Arc::new(Mutex::new(0usize));
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(FailingHook));
    runner.register(Box::new(MockHook::new(count.clone(), None)));

    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Continue);
    assert_eq!(*count.lock().unwrap(), 1); // second hook still runs
}

#[tokio::test]
async fn test_hook_runner_no_hooks() {
    let runner = HookRunner::new(VerifyTrigger::Always);
    let result = runner.run_hooks(&completed_step()).await;
    assert_eq!(result, HookResult::Continue);
}

// ---------------------------------------------------------------------------
// VerificationHook tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_verification_hook_success() {
    let adapter = MockSpawnAdapter::new(vec![Ok(SubAgentResult {
        step_index: 0,
        status: ExecutionStepStatus::Completed,
        summary: "verified".into(),
        changed_files: vec![],
        error_message: None,
    })]);
    let hook = VerificationHook::new(adapter);

    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(result, HookResult::Continue);
}

#[tokio::test]
async fn test_verification_hook_spawn_failure() {
    let adapter = MockSpawnAdapter::new(vec![Err(ExecutionError::SpawnFailed {
        message: "boom".into(),
    })]);
    let hook = VerificationHook::new(adapter);

    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(
        result,
        HookResult::Block("verification failed: spawn failed: boom".into())
    );
}

// ---------------------------------------------------------------------------
// NotifyHook tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_notify_hook_calls_callback() {
    let called = Arc::new(Mutex::new(Vec::<(usize, String)>::new()));
    let called_clone = called.clone();

    let hook = NotifyHook::new(move |idx, summary| {
        let called = called_clone.clone();
        async move {
            called.lock().unwrap().push((idx, summary));
            Ok(())
        }
    });

    let step = completed_step();
    let result = hook.execute(&step).await.unwrap();
    assert_eq!(result, HookResult::Continue);

    let calls = called.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, 0);
    assert_eq!(calls[0].1, "done");
}

#[tokio::test]
async fn test_notify_hook_error_is_non_blocking() {
    let hook = NotifyHook::new(|_idx, _summary| async { Err("callback error".into()) });

    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(result, HookResult::Continue);
}

// ---------------------------------------------------------------------------
// CustomHook tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_custom_hook_success() {
    let hook = CustomHook::new("echo hello".into(), Duration::from_secs(5));
    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(result, HookResult::Continue);
}

#[tokio::test]
async fn test_custom_hook_failure_is_non_blocking() {
    let hook = CustomHook::new("exit 1".into(), Duration::from_secs(5));
    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(result, HookResult::Continue);
}

#[tokio::test]
async fn test_custom_hook_timeout_is_non_blocking() {
    let hook = CustomHook::new("sleep 10".into(), Duration::from_millis(50));
    let result = hook.execute(&completed_step()).await.unwrap();
    assert_eq!(result, HookResult::Continue);
}

#[tokio::test]
async fn test_custom_hook_invalid_command() {
    // sh -c with nonexistent command spawns successfully but exits non-zero
    let hook = CustomHook::new("nonexistent_command_xyz".into(), Duration::from_secs(5));
    let result = hook.execute(&completed_step()).await.unwrap();
    // Non-blocking per plan: command failure does not block step completion
    assert_eq!(result, HookResult::Continue);
}

// ---------------------------------------------------------------------------
// HookResult Display
// ---------------------------------------------------------------------------

#[test]
fn test_hook_result_display_continue() {
    assert_eq!(format!("{}", HookResult::Continue), "continue");
}

#[test]
fn test_hook_result_display_block() {
    assert_eq!(
        format!("{}", HookResult::Block("reason".into())),
        "block: reason"
    );
}
