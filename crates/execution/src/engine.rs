//! Core execution engine — orchestrates step-by-step execution with
//! retry logic and state management.
//!
//! The engine drives the main loop: take a pending step, mark it
//! in-progress, dispatch via a [`SpawnAdapter`], update state, and
//! repeat. Retries are controlled by [`ExecutionConfig`].

use std::sync::{Arc, Mutex};

use closeclaw_common::{ExecutionStepStatus, PlanState};

use crate::error::ExecutionError;
use crate::event::ExecutionEvent;
use crate::spawn::SpawnAdapter;
use crate::types::ExecutionConfig;

/// Result of executing a single step.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Index of the step (0-based).
    pub step_index: usize,
    /// Human-readable description of the step.
    pub description: String,
    /// Final status after execution.
    pub status: ExecutionStepStatus,
    /// Summary returned by the executor.
    pub summary: String,
    /// Files changed during execution.
    pub changed_files: Vec<String>,
    /// Error message if the step failed.
    pub error_message: Option<String>,
    /// Number of attempts made (1 = no retry).
    pub attempts: u32,
}

/// Report produced after a full execution cycle.
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    /// Per-step results in order.
    pub steps: Vec<StepResult>,
    /// Whether every step completed successfully.
    pub all_completed: bool,
    /// Index of the first step that failed, if any.
    pub failed_step: Option<usize>,
}

/// Core execution engine — drives the step-by-step scheduling loop.
///
/// Generic over `S: SpawnAdapter` so the actual dispatch mechanism can
/// be swapped (real spawn, mock, inline).
pub struct ExecutionEngine<S> {
    /// Shared plan state, protected by a mutex for interior mutability.
    plan_state: Arc<Mutex<PlanState>>,
    /// Execution configuration (mode, retries, etc.).
    config: ExecutionConfig,
    /// Adapter used to dispatch step execution.
    adapter: S,
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Create a new execution engine.
    pub fn new(plan_state: Arc<Mutex<PlanState>>, config: ExecutionConfig, adapter: S) -> Self {
        Self {
            plan_state,
            config,
            adapter,
        }
    }

    /// Execute all provided steps sequentially and return a report.
    ///
    /// Initializes the plan state with the given step descriptions,
    /// then processes each step in order. On failure, retries are
    /// attempted according to the configured max retries and strategy.
    pub async fn execute(&self, steps: &[String]) -> Result<ExecutionReport, ExecutionError> {
        let steps_owned: Vec<String> = steps.to_vec();

        // Initialize plan state with execution steps
        {
            let mut state = self.plan_state.lock().expect("plan state lock poisoned");
            state.init_execution_steps(steps_owned.clone());
        }

        let mut results: Vec<StepResult> = Vec::new();
        let mut failed_step: Option<usize> = None;

        for (i, description) in steps_owned.iter().enumerate() {
            let step_result = self
                .execute_step_with_retries(i, description, &results)
                .await?;
            let is_failed = matches!(step_result.status, ExecutionStepStatus::Failed);
            results.push(step_result);

            if is_failed {
                failed_step = Some(i);
                break;
            }
        }

        let all_completed = failed_step.is_none() && results.len() == steps.len();

        if all_completed {
            tracing::info!("all {} steps completed successfully", steps.len());
        } else if let Some(idx) = failed_step {
            tracing::warn!("execution stopped at step {idx}");
        }

        Ok(ExecutionReport {
            steps: results,
            all_completed,
            failed_step,
        })
    }

    /// Execute a single step, retrying on failure up to `max_retries`.
    ///
    /// Returns the final [`StepResult`] after all attempts are exhausted.
    async fn execute_step_with_retries(
        &self,
        step_index: usize,
        description: &str,
        _previous: &[StepResult],
    ) -> Result<StepResult, ExecutionError> {
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_retries + 1;

        loop {
            attempt += 1;

            // Mark step as in-progress
            self.mark_step_status(step_index, ExecutionStepStatus::InProgress)?;

            tracing::info!(step_index, attempt, max_attempts, "dispatching step");

            // Dispatch to the adapter
            let result = self.adapter.spawn_run(description, "").await;

            match result {
                Ok(sub_result) => {
                    let final_status = sub_result.status;

                    if matches!(final_status, ExecutionStepStatus::Completed) {
                        self.mark_step_status(step_index, ExecutionStepStatus::Completed)?;
                        tracing::info!(step_index, "step completed");

                        return Ok(StepResult {
                            step_index,
                            description: description.to_string(),
                            status: final_status,
                            summary: sub_result.summary,
                            changed_files: sub_result.changed_files,
                            error_message: sub_result.error_message,
                            attempts: attempt,
                        });
                    }

                    // Sub-agent reported failure
                    tracing::warn!(step_index, attempt, "step failed per sub-agent result");

                    if attempt < max_attempts {
                        self.emit_event(ExecutionEvent::RetryTriggered {
                            step_index,
                            attempt: attempt + 1,
                        });
                        // Mark failed to allow next InProgress transition
                        self.mark_step_status(step_index, ExecutionStepStatus::Failed)?;
                        continue;
                    }

                    // Retries exhausted — step already marked Failed by loop
                    return Ok(StepResult {
                        step_index,
                        description: description.to_string(),
                        status: ExecutionStepStatus::Failed,
                        summary: sub_result.summary,
                        changed_files: sub_result.changed_files,
                        error_message: sub_result.error_message,
                        attempts: attempt,
                    });
                }
                Err(e) => {
                    tracing::error!(step_index, attempt, error = %e, "spawn error");

                    if attempt < max_attempts {
                        self.emit_event(ExecutionEvent::RetryTriggered {
                            step_index,
                            attempt: attempt + 1,
                        });
                        self.mark_step_status(step_index, ExecutionStepStatus::Failed)?;
                        continue;
                    }

                    // Retries exhausted — step already marked Failed by loop
                    return Ok(StepResult {
                        step_index,
                        description: description.to_string(),
                        status: ExecutionStepStatus::Failed,
                        summary: String::new(),
                        changed_files: Vec::new(),
                        error_message: Some(e.to_string()),
                        attempts: attempt,
                    });
                }
            }
        }
    }

    /// Apply a status transition to the given step in plan state.
    fn mark_step_status(
        &self,
        step_index: usize,
        status: ExecutionStepStatus,
    ) -> Result<(), ExecutionError> {
        let mut state = self.plan_state.lock().expect("plan state lock poisoned");

        // Set current_step before transitioning to InProgress
        if matches!(status, ExecutionStepStatus::InProgress) {
            state.current_step = Some(step_index);
        }

        state
            .apply_transition(step_index, status)
            .map_err(|e| ExecutionError::InvalidResult {
                message: format!("state transition failed: {e}"),
            })?;

        Ok(())
    }

    /// Emit an execution event (currently logs only).
    fn emit_event(&self, event: ExecutionEvent) {
        tracing::info!(?event, "execution event");
    }

    /// Access a snapshot of the current plan state.
    pub fn plan_state_snapshot(&self) -> PlanState {
        self.plan_state
            .lock()
            .expect("plan state lock poisoned")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
    use async_trait::async_trait;
    use std::sync::Arc;

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

        async fn spawn_session(
            &self,
            _task: &str,
            _context: &str,
        ) -> Result<String, ExecutionError> {
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
        ExecutionEngine::new(plan_state, default_config(), adapter)
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
        let engine = ExecutionEngine::new(plan_state, config, adapter);
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
        let engine = ExecutionEngine::new(plan_state, config, adapter);
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
        let engine = ExecutionEngine::new(plan_state.clone(), default_config(), adapter);
        let _ = engine.execute(&["only step".into()]).await.unwrap();

        let state = plan_state.lock().unwrap();
        assert_eq!(state.execution_steps.len(), 1);
        assert!(matches!(
            state.execution_steps[0].status,
            ExecutionStepStatus::Completed
        ));
    }
}
