//! Cross-module integration tests for Step 1.6.
//!
//! Verifies the complete flow: step completion → hook triggers.
//! Also tests retry scenarios where hooks only fire on final completion.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_execution::error::ExecutionError;
use closeclaw_execution::event::ExecutionEvent;
use closeclaw_execution::hook::{HookError, HookResult, HookRunner, NotifyHook, StepHook};
use closeclaw_execution::spawn::SpawnAdapter;
use closeclaw_execution::types::{ExecutionConfig, ExecutionMode, SubAgentResult, VerifyTrigger};
use closeclaw_execution::ExecutionStepStatus;
use closeclaw_execution::{ExecutionEngine, StepResult};

// ── Mock adapters ────────────────────────────────────────────────────────

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

// ── Mock hooks ───────────────────────────────────────────────────────────

#[allow(dead_code)]
struct RecordingHook {
    call_count: Arc<AtomicUsize>,
    step_indices: Arc<Mutex<Vec<usize>>>,
}

#[allow(dead_code)]
impl RecordingHook {
    fn new() -> Self {
        Self {
            call_count: Arc::new(AtomicUsize::new(0)),
            step_indices: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn step_indices(&self) -> Vec<usize> {
        self.step_indices.lock().unwrap().clone()
    }
}

#[async_trait]
impl StepHook for RecordingHook {
    async fn execute(&self, step: &StepResult) -> Result<HookResult, HookError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.step_indices.lock().unwrap().push(step.step_index);
        Ok(HookResult::Continue)
    }
}

/// Hook that always returns Block.
struct BlockingHook;

#[async_trait]
impl StepHook for BlockingHook {
    async fn execute(&self, _step: &StepResult) -> Result<HookResult, HookError> {
        Ok(HookResult::Block("intentional block".into()))
    }
}

/// Recording hook that only counts calls (no step_indices tracking).
struct RecordingHookSimple {
    call_count: Arc<AtomicUsize>,
}

#[async_trait]
impl StepHook for RecordingHookSimple {
    async fn execute(&self, _step: &StepResult) -> Result<HookResult, HookError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(HookResult::Continue)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn spawn_per_step_config() -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
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

#[allow(dead_code)]
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
    ExecutionEngine::new(config, adapter, None)
}

// ===========================================================================
// Integration Test 1: Full flow
// Steps completed → hook triggers
// ===========================================================================

#[tokio::test]
async fn test_full_flow_completed_hook_notifies_system_prompt() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let hook_indices: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let hook = RecordingHook {
        call_count: hook_count.clone(),
        step_indices: hook_indices.clone(),
    };

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(hook));

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "implement feature A")),
        Ok(success_result(1, "write tests")),
    ]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine
        .execute(&["implement feature A".into(), "write tests".into()])
        .await
        .unwrap();

    // 1. Both steps completed
    assert!(report.all_completed);
    assert_eq!(report.steps.len(), 2);

    // 2. Hook was called for each completed step
    assert_eq!(hook_count.load(Ordering::SeqCst), 2);
    let indices = hook_indices.lock().unwrap();
    assert_eq!(*indices, vec![0, 1]);

    // 3. Hook events are recorded
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookExecuted { step_index: 0 })));
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookExecuted { step_index: 1 })));
}

// ===========================================================================
// Integration Test 2: Hook fires on each completed step
// ===========================================================================

#[tokio::test]
async fn test_hook_fires_on_each_completed_step() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let hook_indices: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let hook = RecordingHook {
        call_count: hook_count.clone(),
        step_indices: hook_indices.clone(),
    };

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(hook));

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "step 0 done")),
        Ok(success_result(1, "step 1 done")),
    ]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    assert!(report.all_completed);

    // Hook fired for each completed step
    assert_eq!(hook_count.load(Ordering::SeqCst), 2);
    let indices = hook_indices.lock().unwrap();
    assert_eq!(*indices, vec![0, 1]);
}

// ===========================================================================
// Integration Test 3: Hook + Notifier coordination
// ===========================================================================

#[tokio::test]
async fn test_hook_and_notifier_coordination() {
    // Track execution order: hooks and notifier calls
    let event_order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let event_order_clone = event_order.clone();
    let hook = NotifyHook::new(move |idx, summary| {
        let order = event_order_clone.clone();
        async move {
            order.lock().unwrap().push(format!("hook:{idx}:{summary}"));
            Ok(())
        }
    });

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(hook));

    let adapter = SequenceMock::new(vec![Ok(success_result(0, "step done"))]);
    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let _report = engine.execute(&["step A".into()]).await.unwrap();

    let order = event_order.lock().unwrap();
    assert!(
        order.iter().any(|e| e.starts_with("hook:0:")),
        "hook callback should have been recorded"
    );
}

// ===========================================================================
// Integration Test 4: Hook failure does not block execution
// ===========================================================================

#[tokio::test]
async fn test_hook_failure_does_not_block_notifier() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook));
    runner.register(Box::new(RecordingHookSimple {
        call_count: hook_count.clone(),
    }));

    let adapter = SequenceMock::new(vec![Ok(success_result(0, "done"))]);
    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine.execute(&["step".into()]).await.unwrap();

    // Step still completed
    assert!(report.all_completed);
    // Hook failure is recorded as HookFailed event
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookFailed { step_index: 0, .. })));
    // Second hook (after failure) should NOT run (block stops subsequent)
    assert_eq!(hook_count.load(Ordering::SeqCst), 0);
}

// ===========================================================================
// Integration Test 5: NonTrivial trigger + hook
// ===========================================================================

#[tokio::test]
async fn test_nontrivial_hook_with_progress_tracking() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let hook = RecordingHook {
        call_count: hook_count.clone(),
        step_indices: Arc::new(Mutex::new(Vec::new())),
    };

    let mut runner = HookRunner::new(VerifyTrigger::NonTrivial);
    runner.register(Box::new(hook));

    // Step 0 has changed files (non-trivial), step 1 does not (trivial)
    let adapter = SequenceMock::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Completed,
            summary: "implement".into(),
            changed_files: vec!["src/foo.rs".into()],
            error_message: None,
        }),
        Ok(SubAgentResult {
            step_index: 1,
            status: ExecutionStepStatus::Completed,
            summary: "document".into(),
            changed_files: vec![], // trivial — no files changed
            error_message: None,
        }),
    ]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine
        .execute(&["implement".into(), "document".into()])
        .await
        .unwrap();

    assert!(report.all_completed);
    // Hook only fired for step 0 (non-trivial), not step 1 (trivial)
    assert_eq!(hook_count.load(Ordering::SeqCst), 1);
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookExecuted { step_index: 0, .. })));
    assert!(!report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookExecuted { step_index: 1, .. })));
}

// ===========================================================================
// Integration Test 6: Hook failure does not prevent step completion
// ===========================================================================

#[tokio::test]
async fn test_hook_failure_does_not_prevent_step_completion() {
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook));

    let adapter = SequenceMock::new(vec![Ok(success_result(0, "done"))]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine.execute(&["step".into()]).await.unwrap();

    // Step completed despite hook blocking
    assert!(report.all_completed);
    // Hook was called and blocked
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookFailed { step_index: 0, .. })));
}

// ===========================================================================
// Integration Test 7: Multi-step with mixed hook results
// ===========================================================================

#[tokio::test]
async fn test_multi_step_mixed_hook_results() {
    let hook0_count = Arc::new(AtomicUsize::new(0));

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook)); // blocks
    runner.register(Box::new(RecordingHookSimple {
        call_count: hook0_count.clone(),
    })); // blocked

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "step 0 done")),
        // Step 1 should never be executed due to hook block on step 0
    ]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    // Hook blocked on step 0, so execution stopped — not all steps completed
    assert!(!report.all_completed);
    // Only step 0 executed (hook blocked, stopped before step 1)
    assert_eq!(report.steps.len(), 1);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Completed
    ));
    // Hook block is recorded on the step result
    assert_eq!(
        report.steps[0].hook_blocked.as_deref(),
        Some("intentional block")
    );
    // hook_blocked flag on report
    assert!(report.hook_blocked);
    // HookFailed event recorded for step 0
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookFailed { step_index: 0, .. })));
    // Step 1 never executed, so no HookFailed for step 1
    assert!(!report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookFailed { step_index: 1, .. })));
    // The second hook never ran (blocked by first)
    assert_eq!(hook0_count.load(Ordering::SeqCst), 0);
}

// ===========================================================================
// Integration Test 8: Sub-agent Skipped status → treated as failure
// ===========================================================================

#[tokio::test]
async fn test_sub_agent_skipped_treated_as_failure() {
    let adapter = SequenceMock::new(vec![
        Ok(SubAgentResult {
            step_index: 0,
            status: ExecutionStepStatus::Skipped,
            summary: "skipped".into(),
            changed_files: vec![],
            error_message: None,
        }),
        // Step 1 should never be reached
    ]);
    let engine = new_engine_with_config(adapter, spawn_per_step_config());
    let report = engine
        .execute(&["step 0".into(), "step 1".into()])
        .await
        .unwrap();

    // Skipped is treated as failure — execution stops
    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(0));
    assert_eq!(report.steps.len(), 1);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Failed
    ));
}

// ===========================================================================
// Integration Test 10: Sub-agent Skipped stops execution
// ===========================================================================

#[tokio::test]
async fn test_sub_agent_skipped_stops_execution() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let hook_indices: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let hook = RecordingHook {
        call_count: hook_count.clone(),
        step_indices: hook_indices.clone(),
    };

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(hook));

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "done")),
        Ok(SubAgentResult {
            step_index: 1,
            status: ExecutionStepStatus::Skipped,
            summary: "skipped".into(),
            changed_files: vec![],
            error_message: None,
        }),
        // Step 2 should never be reached
    ]);

    let engine = ExecutionEngine::with_hook_runner(spawn_per_step_config(), adapter, runner, None);

    let report = engine
        .execute(&["s0".into(), "s1".into(), "s2".into()])
        .await
        .unwrap();

    // Step 0 completed, step 1 skipped (treated as failure), execution stops
    assert!(!report.all_completed);
    assert_eq!(report.failed_step, Some(1));
    assert_eq!(report.steps.len(), 2);
    assert!(matches!(
        report.steps[0].status,
        ExecutionStepStatus::Completed
    ));
    assert!(matches!(
        report.steps[1].status,
        ExecutionStepStatus::Failed
    ));

    // Hook fired only for completed step 0 (skipped step 1 is treated as failure)
    assert_eq!(hook_count.load(Ordering::SeqCst), 1);
    let indices = hook_indices.lock().unwrap();
    assert_eq!(*indices, vec![0]);
}
