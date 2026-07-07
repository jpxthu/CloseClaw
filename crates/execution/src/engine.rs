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
use crate::types::{ExecutionConfig, ExecutionMode};

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
    /// Events emitted during execution.
    pub events: Vec<ExecutionEvent>,
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

        // Dispatch based on execution mode
        match self.config.mode {
            ExecutionMode::SpawnAllSteps => self.execute_spawn_all(&steps_owned).await,
            ExecutionMode::SpawnPerStep | ExecutionMode::Inline => {
                self.execute_step_by_step(&steps_owned).await
            }
        }
    }

    /// Execute step-by-step (SpawnPerStep / Inline mode).
    async fn execute_step_by_step(
        &self,
        steps_owned: &[String],
    ) -> Result<ExecutionReport, ExecutionError> {
        let mut results: Vec<StepResult> = Vec::new();
        let mut events: Vec<ExecutionEvent> = Vec::new();
        let mut failed_step: Option<usize> = None;

        for (i, description) in steps_owned.iter().enumerate() {
            let step_result = self
                .execute_step_with_retries(i, description, &results, &mut events)
                .await?;
            let is_failed = matches!(step_result.status, ExecutionStepStatus::Failed);
            results.push(step_result);

            if is_failed {
                failed_step = Some(i);
                break;
            }
        }

        let all_completed = failed_step.is_none() && results.len() == steps_owned.len();

        if all_completed {
            tracing::info!("all {} steps completed successfully", steps_owned.len());
            events.push(ExecutionEvent::AllCompleted);
        } else if let Some(idx) = failed_step {
            tracing::warn!("execution stopped at step {idx}");
        }

        Ok(ExecutionReport {
            steps: results,
            all_completed,
            failed_step,
            events,
        })
    }

    /// Execute all steps in a single spawn (SpawnAllSteps mode).
    ///
    /// The adapter receives all step descriptions in a single `spawn_run`
    /// call. Steps 1+ are tracked via `StepResult` only (not through
    /// PlanState transitions) because the state machine enforces sequential
    /// `current_step` advancement and doesn't support batch transitions.
    async fn execute_spawn_all(
        &self,
        steps_owned: &[String],
    ) -> Result<ExecutionReport, ExecutionError> {
        let mut events: Vec<ExecutionEvent> = Vec::new();
        let task = steps_owned.join("\n");
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_retries + 1;

        // Emit StepStarted for all steps
        for i in 0..steps_owned.len() {
            Self::emit_event(&mut events, ExecutionEvent::StepStarted { step_index: i });
        }

        loop {
            attempt += 1;

            // Mark step 0 as InProgress (only step 0 goes through PlanState)
            self.mark_step_status(0, ExecutionStepStatus::InProgress)?;

            let result = self.adapter.spawn_run(&task, "").await;

            match result {
                Ok(sub_result) => {
                    let mut results: Vec<StepResult> = Vec::new();
                    let mut failed_step: Option<usize> = None;

                    // Process step 0 through PlanState
                    let step0_status = sub_result.status;
                    let step0_failed = matches!(step0_status, ExecutionStepStatus::Failed);

                    if step0_failed {
                        self.mark_step_status(0, ExecutionStepStatus::Failed)?;
                        Self::emit_event(
                            &mut events,
                            ExecutionEvent::StepFailed {
                                step_index: 0,
                                error_message: sub_result.error_message.clone().unwrap_or_default(),
                            },
                        );
                        failed_step = Some(0);
                    } else {
                        self.mark_step_status(0, ExecutionStepStatus::Completed)?;
                        Self::emit_event(
                            &mut events,
                            ExecutionEvent::StepCompleted {
                                step_index: 0,
                                summary: sub_result.summary.clone(),
                            },
                        );
                    }

                    results.push(StepResult {
                        step_index: 0,
                        description: steps_owned[0].clone(),
                        status: step0_status,
                        summary: sub_result.summary.clone(),
                        changed_files: sub_result.changed_files.clone(),
                        error_message: sub_result.error_message.clone(),
                        attempts: attempt,
                    });

                    // Process steps 1+ (tracked via StepResult only)
                    for (i, description) in steps_owned.iter().enumerate().skip(1) {
                        let is_failed = step0_failed && failed_step == Some(0);
                        let status = if is_failed {
                            ExecutionStepStatus::Failed
                        } else {
                            ExecutionStepStatus::Completed
                        };

                        if is_failed {
                            Self::emit_event(
                                &mut events,
                                ExecutionEvent::StepFailed {
                                    step_index: i,
                                    error_message: sub_result
                                        .error_message
                                        .clone()
                                        .unwrap_or_default(),
                                },
                            );
                        } else {
                            Self::emit_event(
                                &mut events,
                                ExecutionEvent::StepCompleted {
                                    step_index: i,
                                    summary: sub_result.summary.clone(),
                                },
                            );
                        }

                        results.push(StepResult {
                            step_index: i,
                            description: description.clone(),
                            status,
                            summary: sub_result.summary.clone(),
                            changed_files: sub_result.changed_files.clone(),
                            error_message: sub_result.error_message.clone(),
                            attempts: attempt,
                        });
                    }

                    let all_completed = failed_step.is_none() && results.len() == steps_owned.len();

                    if all_completed {
                        events.push(ExecutionEvent::AllCompleted);
                    }

                    return Ok(ExecutionReport {
                        steps: results,
                        all_completed,
                        failed_step,
                        events,
                    });
                }
                Err(e) => {
                    if attempt < max_attempts {
                        // Mark step 0 as Failed for retry
                        self.mark_step_status(0, ExecutionStepStatus::Failed)?;
                        Self::emit_event(
                            &mut events,
                            ExecutionEvent::RetryTriggered {
                                step_index: 0,
                                attempt: attempt + 1,
                            },
                        );
                        continue;
                    }

                    // Retries exhausted — mark step 0 as Failed
                    self.mark_step_status(0, ExecutionStepStatus::Failed)?;
                    let mut results: Vec<StepResult> = Vec::new();
                    for (i, description) in steps_owned.iter().enumerate() {
                        Self::emit_event(
                            &mut events,
                            ExecutionEvent::StepFailed {
                                step_index: i,
                                error_message: e.to_string(),
                            },
                        );
                        results.push(StepResult {
                            step_index: i,
                            description: description.clone(),
                            status: ExecutionStepStatus::Failed,
                            summary: String::new(),
                            changed_files: Vec::new(),
                            error_message: Some(e.to_string()),
                            attempts: attempt,
                        });
                    }
                    return Ok(ExecutionReport {
                        steps: results,
                        all_completed: false,
                        failed_step: Some(0),
                        events,
                    });
                }
            }
        }
    }

    /// Execute a single step, retrying on failure up to `max_retries`.
    ///
    /// Returns the final [`StepResult`] after all attempts are exhausted.
    async fn execute_step_with_retries(
        &self,
        step_index: usize,
        description: &str,
        _previous: &[StepResult],
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<StepResult, ExecutionError> {
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_retries + 1;

        loop {
            attempt += 1;

            // Emit StepStarted only on first attempt
            if attempt == 1 {
                Self::emit_event(events, ExecutionEvent::StepStarted { step_index });
            }
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

                        Self::emit_event(
                            events,
                            ExecutionEvent::StepCompleted {
                                step_index,
                                summary: sub_result.summary.clone(),
                            },
                        );

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
                        Self::emit_event(
                            events,
                            ExecutionEvent::RetryTriggered {
                                step_index,
                                attempt: attempt + 1,
                            },
                        );
                        // Mark failed to allow next InProgress transition
                        self.mark_step_status(step_index, ExecutionStepStatus::Failed)?;
                        continue;
                    }

                    // Retries exhausted
                    Self::emit_event(
                        events,
                        ExecutionEvent::StepFailed {
                            step_index,
                            error_message: sub_result.error_message.clone().unwrap_or_default(),
                        },
                    );
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
                        Self::emit_event(
                            events,
                            ExecutionEvent::RetryTriggered {
                                step_index,
                                attempt: attempt + 1,
                            },
                        );
                        self.mark_step_status(step_index, ExecutionStepStatus::Failed)?;
                        continue;
                    }

                    // Retries exhausted
                    Self::emit_event(
                        events,
                        ExecutionEvent::StepFailed {
                            step_index,
                            error_message: e.to_string(),
                        },
                    );
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

    /// Emit an execution event — logs it and appends to the events list.
    fn emit_event(events: &mut Vec<ExecutionEvent>, event: ExecutionEvent) {
        tracing::info!(?event, "execution event");
        events.push(event);
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
