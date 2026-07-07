//! Cross-module integration tests for Step 1.6.
//!
//! Verifies the complete flow: step completion → hook triggers →
//! progress notification → system prompt update. Also tests retry
//! scenarios where hooks only fire on final completion.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::{ExecutionStepStatus, PlanState, PlanStateNotifier};
use closeclaw_execution::error::ExecutionError;
use closeclaw_execution::event::ExecutionEvent;
use closeclaw_execution::hook::{HookError, HookResult, HookRunner, NotifyHook, StepHook};
use closeclaw_execution::spawn::SpawnAdapter;
use closeclaw_execution::types::{
    ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger,
};
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

// ── Mock PlanStateNotifier that tracks calls ──────────────────────────────

#[allow(dead_code)]
struct RecordingNotifier {
    summaries: Arc<Mutex<Vec<String>>>,
    call_count: Arc<AtomicUsize>,
}

#[allow(dead_code)]
impl RecordingNotifier {
    fn new() -> Self {
        Self {
            summaries: Arc::new(Mutex::new(Vec::new())),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn summaries(&self) -> Vec<String> {
        self.summaries.lock().unwrap().clone()
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PlanStateNotifier for RecordingNotifier {
    async fn on_progress_changed(&self, progress_summary: &str) {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.summaries
            .lock()
            .unwrap()
            .push(progress_summary.to_string());
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

// ── Simulates session layer's system_appends behavior ────────────────────

/// Simulates the session layer's system_appends behavior:
/// receives progress summaries via the notifier and tracks them
/// as if they were appended to the system prompt.
struct MockSystemAppends {
    appends: Mutex<Vec<String>>,
}

impl MockSystemAppends {
    fn new() -> Self {
        Self {
            appends: Mutex::new(Vec::new()),
        }
    }

    fn appends(&self) -> Vec<String> {
        self.appends.lock().unwrap().clone()
    }
}

#[async_trait]
impl PlanStateNotifier for MockSystemAppends {
    async fn on_progress_changed(&self, progress_summary: &str) {
        let mut appends = self.appends.lock().unwrap();
        // Simulate the session layer: replace existing progress entry
        // (identified by prefix) or append new one.
        let tagged = format!("__progress__:{}", progress_summary);
        if let Some(slot) = appends.iter_mut().find(|s| s.starts_with("__progress__:")) {
            *slot = tagged;
        } else {
            appends.push(tagged);
        }
    }
}

/// Notifier that appends to the shared order vec for tracking execution order.
struct TrackingNotifier {
    order: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PlanStateNotifier for TrackingNotifier {
    async fn on_progress_changed(&self, progress_summary: &str) {
        self.order
            .lock()
            .unwrap()
            .push(format!("notifier:{progress_summary}"));
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn spawn_per_step_config(max_retries: u32) -> ExecutionConfig {
    ExecutionConfig {
        mode: ExecutionMode::SpawnPerStep,
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

// ===========================================================================
// Integration Test 1: Full flow
// Steps completed → hook triggers → progress notification → system prompt update
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

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "implement feature A")),
        Ok(success_result(1, "write tests")),
    ]);

    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state.clone(),
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

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

    // 4. System appends reflect progress updates
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1); // only one entry (replaced each time)
                                  // Final state should show step 2/2 completed
    assert!(appends[0].contains("Step 2/2: completed"));
    assert!(appends[0].starts_with("__progress__:"));

    // 5. Plan state is updated
    let state = plan_state.lock().unwrap();
    assert!(matches!(
        state.execution_steps[0].status,
        ExecutionStepStatus::Completed
    ));
    assert!(matches!(
        state.execution_steps[1].status,
        ExecutionStepStatus::Completed
    ));
}

// ===========================================================================
// Integration Test 2: Retry scenario
// failed → retry → completed, hook only fires on final completed
// ===========================================================================

#[tokio::test]
async fn test_retry_hook_only_fires_on_final_completed() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let hook_indices: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let hook = RecordingHook {
        call_count: hook_count.clone(),
        step_indices: hook_indices.clone(),
    };

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(hook));

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

    // Step 0: fails first, then succeeds on retry
    let adapter = SequenceMock::new(vec![
        Ok(failed_result(0, "transient error")),
        Ok(success_result(0, "recovered")),
        Ok(success_result(1, "all done")),
    ]);

    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state.clone(),
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

    let report = engine
        .execute(&["flaky step".into(), "reliable step".into()])
        .await
        .unwrap();

    assert!(report.all_completed);

    // Hook only fired for completed steps, NOT for the failed intermediate attempt
    // Step 0 completed on retry (attempt 2), step 1 completed on first attempt
    assert_eq!(hook_count.load(Ordering::SeqCst), 2);
    let indices = hook_indices.lock().unwrap();
    assert_eq!(*indices, vec![0, 1]);

    // Progress shows final state
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].contains("Step 2/2: completed"));
}

// ===========================================================================
// Integration Test 3: Hook + Notifier coordination
// Serial hooks execute before the final step completes and notifies
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

    let notifier_order = event_order.clone();
    let tracking_notifier = Arc::new(TrackingNotifier {
        order: notifier_order,
    });

    let adapter = SequenceMock::new(vec![Ok(success_result(0, "step done"))]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state,
        spawn_per_step_config(3),
        adapter,
        tracking_notifier,
        runner,
        None,
    );

    let _report = engine.execute(&["step A".into()]).await.unwrap();

    let order = event_order.lock().unwrap();
    // Hook fires during complete_step, then notifier fires after mark_step_status
    // Order: hook:0:step done (hook callback), notifier (progress update)
    assert!(
        order.iter().any(|e| e.starts_with("hook:0:")),
        "hook callback should have been recorded"
    );
}

// ===========================================================================
// Integration Test 4: Hook failure does not block notification
// ===========================================================================

#[tokio::test]
async fn test_hook_failure_does_not_block_notifier() {
    let hook_count = Arc::new(AtomicUsize::new(0));
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook));
    runner.register(Box::new(RecordingHookSimple {
        call_count: hook_count.clone(),
    }));

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

    let adapter = SequenceMock::new(vec![Ok(success_result(0, "done"))]);
    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state,
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

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
    // Notifier was still called (progress updated)
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].contains("completed"));
}

// ===========================================================================
// Integration Test 5: NonTrivial trigger + hook + progress
// Only non-trivial steps (with changed_files) trigger hooks
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

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

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

    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state,
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

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
    // Progress still updated for both steps
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].contains("Step 2/2: completed"));
}

// ===========================================================================
// Integration Test 6: Retry with hook failure
// Step fails, hook fails during retry completion, but step still completes
// ===========================================================================

#[tokio::test]
async fn test_retry_with_hook_failure_still_completes() {
    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook));

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

    let adapter = SequenceMock::new(vec![
        Ok(failed_result(0, "transient")),
        Ok(success_result(0, "recovered")),
    ]);

    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state.clone(),
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

    let report = engine.execute(&["step".into()]).await.unwrap();

    // Step completed despite hook failure
    assert!(report.all_completed);
    // Hook was called (on final completed) and blocked
    assert!(report
        .events
        .iter()
        .any(|e| matches!(e, ExecutionEvent::HookFailed { step_index: 0, .. })));
    // Progress updated
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].contains("completed"));
}

// ===========================================================================
// Integration Test 7: Multi-step with mixed hook results
// Hook blocks on step 0 (Continue recorded as HookFailed), step 1 hooks succeed
// ===========================================================================

#[tokio::test]
async fn test_multi_step_mixed_hook_results() {
    let hook0_count = Arc::new(AtomicUsize::new(0));

    let mut runner = HookRunner::new(VerifyTrigger::Always);
    runner.register(Box::new(BlockingHook)); // blocks
    runner.register(Box::new(RecordingHookSimple {
        call_count: hook0_count.clone(),
    })); // blocked

    let system_appends = Arc::new(MockSystemAppends::new());
    let notifier: Arc<dyn PlanStateNotifier> = system_appends.clone();

    let adapter = SequenceMock::new(vec![
        Ok(success_result(0, "step 0 done")),
        // Step 1 should never be executed due to hook block on step 0
    ]);

    let plan_state = Arc::new(Mutex::new(PlanState::new()));
    let engine = ExecutionEngine::with_hook_runner(
        plan_state,
        spawn_per_step_config(3),
        adapter,
        notifier,
        runner,
        None,
    );

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
    // Progress shows only step 0 completed (step 1 was never reached)
    let appends = system_appends.appends();
    assert_eq!(appends.len(), 1);
    assert!(appends[0].contains("Step 1/2: completed"));
}
