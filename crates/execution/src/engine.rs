//! Core execution engine — orchestrates step-by-step execution with
//! retry logic and state management.
//!
//! The engine drives the main loop: take a pending step, mark it
//! in-progress, dispatch via a [`SpawnAdapter`], update state, and
//! repeat. Retries are controlled by [`ExecutionConfig`].

use std::sync::{Arc, Mutex};

use closeclaw_common::{ExecutionStepStatus, PlanState, PlanStateNotifier};

use crate::error::ExecutionError;
use crate::event::ExecutionEvent;
use crate::hook::HookRunner;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, SubAgentResult};

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
    /// Notifier for progress changes — called after each status transition.
    notifier: Arc<dyn PlanStateNotifier>,
    /// Optional hook runner for post-step actions.
    hook_runner: Option<HookRunner>,
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Create a new execution engine.
    pub fn new(
        plan_state: Arc<Mutex<PlanState>>,
        config: ExecutionConfig,
        adapter: S,
        notifier: Arc<dyn PlanStateNotifier>,
    ) -> Self {
        Self {
            plan_state,
            config,
            adapter,
            notifier,
            hook_runner: None,
        }
    }

    /// Create a new execution engine with a hook runner.
    pub fn with_hook_runner(
        plan_state: Arc<Mutex<PlanState>>,
        config: ExecutionConfig,
        adapter: S,
        notifier: Arc<dyn PlanStateNotifier>,
        hook_runner: HookRunner,
    ) -> Self {
        Self {
            plan_state,
            config,
            adapter,
            notifier,
            hook_runner: Some(hook_runner),
        }
    }

    /// Execute all provided steps sequentially and return a report.
    pub async fn execute(&self, steps: &[String]) -> Result<ExecutionReport, ExecutionError> {
        let steps_owned: Vec<String> = steps.to_vec();

        {
            let mut state = self.plan_state.lock().expect("plan state lock poisoned");
            state.init_execution_steps(steps_owned.clone());
        }

        match self.config.mode {
            ExecutionMode::SpawnAllSteps => self.execute_spawn_all(&steps_owned).await,
            ExecutionMode::SpawnPerStep | ExecutionMode::Inline => {
                self.execute_step_by_step(&steps_owned).await
            }
        }
    }

    /// Access a snapshot of the current plan state.
    pub fn plan_state_snapshot(&self) -> PlanState {
        self.plan_state
            .lock()
            .expect("plan state lock poisoned")
            .clone()
    }
}

// ---------------------------------------------------------------------------
// Step-by-step execution (SpawnPerStep / Inline)
// ---------------------------------------------------------------------------

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Execute steps one at a time; stop on first failure.
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
}

// ---------------------------------------------------------------------------
// SpawnAllSteps execution
// ---------------------------------------------------------------------------

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Execute all steps in a single spawn (SpawnAllSteps mode).
    async fn execute_spawn_all(
        &self,
        steps_owned: &[String],
    ) -> Result<ExecutionReport, ExecutionError> {
        let mut events: Vec<ExecutionEvent> = Vec::new();
        let task = steps_owned.join("\n");
        let mut attempt: u32 = 0;
        let max_attempts = self.config.max_retries + 1;

        for i in 0..steps_owned.len() {
            Self::emit_event(&mut events, ExecutionEvent::StepStarted { step_index: i });
        }

        loop {
            attempt += 1;
            self.mark_step_status(0, ExecutionStepStatus::InProgress)
                .await?;

            match self.adapter.spawn_run(&task, "").await {
                Ok(sub_result) => {
                    return self
                        .handle_spawn_all_success(steps_owned, sub_result, attempt, &mut events)
                        .await;
                }
                Err(e) => {
                    if attempt < max_attempts {
                        self.mark_step_status(0, ExecutionStepStatus::Failed)
                            .await?;
                        Self::emit_event(
                            &mut events,
                            ExecutionEvent::RetryTriggered {
                                step_index: 0,
                                attempt: attempt + 1,
                            },
                        );
                        continue;
                    }
                    return self
                        .handle_spawn_all_retries_exhausted(steps_owned, e, attempt, &mut events)
                        .await;
                }
            }
        }
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Process a successful spawn result for SpawnAllSteps mode.
    async fn handle_spawn_all_success(
        &self,
        steps_owned: &[String],
        sub_result: SubAgentResult,
        attempt: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<ExecutionReport, ExecutionError> {
        let mut results: Vec<StepResult> = Vec::new();
        let step0_failed = matches!(sub_result.status, ExecutionStepStatus::Failed);
        let failed_step = self
            .process_spawn_all_step0(
                &sub_result,
                attempt,
                step0_failed,
                steps_owned,
                &mut results,
                events,
            )
            .await?;

        self.build_spawn_all_remaining_results(
            steps_owned,
            &sub_result,
            step0_failed,
            attempt,
            &mut results,
            events,
        );

        let all_completed = failed_step.is_none() && results.len() == steps_owned.len();
        if all_completed {
            events.push(ExecutionEvent::AllCompleted);
        }

        Ok(ExecutionReport {
            steps: results,
            all_completed,
            failed_step,
            events: events.clone(),
        })
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Process step 0 result in SpawnAllSteps mode.
    async fn process_spawn_all_step0(
        &self,
        sub_result: &SubAgentResult,
        attempt: u32,
        step0_failed: bool,
        steps_owned: &[String],
        results: &mut Vec<StepResult>,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<usize>, ExecutionError> {
        let mut failed_step = None;
        if step0_failed {
            self.mark_step_status(0, ExecutionStepStatus::Failed)
                .await?;
            Self::emit_event(
                events,
                ExecutionEvent::StepFailed {
                    step_index: 0,
                    error_message: sub_result.error_message.clone().unwrap_or_default(),
                },
            );
            failed_step = Some(0);
        } else {
            self.mark_step_status(0, ExecutionStepStatus::Completed)
                .await?;
            Self::emit_event(
                events,
                ExecutionEvent::StepCompleted {
                    step_index: 0,
                    summary: sub_result.summary.clone(),
                },
            );
        }
        results.push(StepResult {
            step_index: 0,
            description: steps_owned[0].clone(),
            status: sub_result.status,
            summary: sub_result.summary.clone(),
            changed_files: sub_result.changed_files.clone(),
            error_message: sub_result.error_message.clone(),
            attempts: attempt,
        });
        Ok(failed_step)
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Build StepResult entries for steps 1+ in SpawnAllSteps mode.
    fn build_spawn_all_remaining_results(
        &self,
        steps_owned: &[String],
        sub_result: &SubAgentResult,
        step0_failed: bool,
        attempts: u32,
        results: &mut Vec<StepResult>,
        events: &mut Vec<ExecutionEvent>,
    ) {
        for (i, description) in steps_owned.iter().enumerate().skip(1) {
            let status = if step0_failed {
                ExecutionStepStatus::Failed
            } else {
                ExecutionStepStatus::Completed
            };

            if step0_failed {
                Self::emit_event(
                    events,
                    ExecutionEvent::StepFailed {
                        step_index: i,
                        error_message: sub_result.error_message.clone().unwrap_or_default(),
                    },
                );
            } else {
                Self::emit_event(
                    events,
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
                attempts,
            });
        }
    }

    /// Return a failure report when all SpawnAllSteps retries are exhausted.
    async fn handle_spawn_all_retries_exhausted(
        &self,
        steps_owned: &[String],
        error: ExecutionError,
        attempt: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<ExecutionReport, ExecutionError> {
        self.mark_step_status(0, ExecutionStepStatus::Failed)
            .await?;
        let results: Vec<StepResult> = steps_owned
            .iter()
            .enumerate()
            .map(|(i, description)| {
                Self::emit_event(
                    events,
                    ExecutionEvent::StepFailed {
                        step_index: i,
                        error_message: error.to_string(),
                    },
                );
                StepResult {
                    step_index: i,
                    description: description.clone(),
                    status: ExecutionStepStatus::Failed,
                    summary: String::new(),
                    changed_files: Vec::new(),
                    error_message: Some(error.to_string()),
                    attempts: attempt,
                }
            })
            .collect();

        Ok(ExecutionReport {
            steps: results,
            all_completed: false,
            failed_step: Some(0),
            events: events.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Single-step retry execution
// ---------------------------------------------------------------------------

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Execute a single step, retrying on failure up to `max_retries`.
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
            if attempt == 1 {
                Self::emit_event(events, ExecutionEvent::StepStarted { step_index });
            }
            self.mark_step_status(step_index, ExecutionStepStatus::InProgress)
                .await?;
            tracing::info!(step_index, attempt, max_attempts, "dispatching step");

            let result = self
                .dispatch_step(step_index, description, attempt, max_attempts, events)
                .await?;
            if let Some(final_result) = result {
                return Ok(final_result);
            }
        }
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Dispatch a single step attempt and process the result.
    async fn dispatch_step(
        &self,
        step_index: usize,
        description: &str,
        attempt: u32,
        max_attempts: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<StepResult>, ExecutionError> {
        match self.adapter.spawn_run(description, "").await {
            Ok(sub_result) => {
                self.handle_step_spawn_result(
                    step_index,
                    description,
                    sub_result,
                    attempt,
                    max_attempts,
                    events,
                )
                .await
            }
            Err(e) => {
                self.handle_step_spawn_error(
                    step_index,
                    description,
                    e,
                    attempt,
                    max_attempts,
                    events,
                )
                .await
            }
        }
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Process a spawn result for a single step.
    ///
    /// Returns `Some(StepResult)` when the step is final (success or retries
    /// exhausted). Returns `Err` only on fatal errors (e.g. state transition
    /// failure).
    async fn handle_step_spawn_result(
        &self,
        step_index: usize,
        description: &str,
        sub_result: SubAgentResult,
        attempt: u32,
        max_attempts: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<StepResult>, ExecutionError> {
        if matches!(sub_result.status, ExecutionStepStatus::Completed) {
            return self
                .complete_step(step_index, description, &sub_result, attempt, events)
                .await;
        }

        tracing::warn!(step_index, attempt, "step failed per sub-agent result");

        if attempt < max_attempts {
            Self::emit_event(
                events,
                ExecutionEvent::RetryTriggered {
                    step_index,
                    attempt: attempt + 1,
                },
            );
            self.mark_step_status(step_index, ExecutionStepStatus::Failed)
                .await?;
            return Ok(None);
        }

        self.fail_step_final(step_index, &sub_result, attempt, events)
            .await
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Mark a step as completed and build its result.
    async fn complete_step(
        &self,
        step_index: usize,
        description: &str,
        sub_result: &SubAgentResult,
        attempt: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<StepResult>, ExecutionError> {
        self.mark_step_status(step_index, ExecutionStepStatus::Completed)
            .await?;
        tracing::info!(step_index, "step completed");
        Self::emit_event(
            events,
            ExecutionEvent::StepCompleted {
                step_index,
                summary: sub_result.summary.clone(),
            },
        );

        let step_result = self.build_step_result(
            step_index,
            description,
            sub_result.status,
            sub_result,
            attempt,
        );

        self.run_hooks_for_step(&step_result, events).await;

        Ok(Some(step_result))
    }

    /// Emit final failure event and build failed result.
    async fn fail_step_final(
        &self,
        step_index: usize,
        sub_result: &SubAgentResult,
        attempt: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<StepResult>, ExecutionError> {
        Self::emit_event(
            events,
            ExecutionEvent::StepFailed {
                step_index,
                error_message: sub_result.error_message.clone().unwrap_or_default(),
            },
        );
        Ok(Some(self.build_step_result(
            step_index,
            "",
            ExecutionStepStatus::Failed,
            sub_result,
            attempt,
        )))
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Process a spawn error (network/fault) for a single step.
    async fn handle_step_spawn_error(
        &self,
        step_index: usize,
        description: &str,
        error: ExecutionError,
        attempt: u32,
        max_attempts: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<Option<StepResult>, ExecutionError> {
        tracing::error!(step_index, attempt, error = %error, "spawn error");

        if attempt < max_attempts {
            Self::emit_event(
                events,
                ExecutionEvent::RetryTriggered {
                    step_index,
                    attempt: attempt + 1,
                },
            );
            self.mark_step_status(step_index, ExecutionStepStatus::Failed)
                .await?;
            return Ok(None); // allow retry
        }

        Self::emit_event(
            events,
            ExecutionEvent::StepFailed {
                step_index,
                error_message: error.to_string(),
            },
        );
        Ok(Some(self.build_failed_step_result(
            step_index,
            description,
            &error,
            attempt,
        )))
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Build a [`StepResult`] from a [`SubAgentResult`].
    fn build_step_result(
        &self,
        step_index: usize,
        description: &str,
        status: ExecutionStepStatus,
        sub_result: &SubAgentResult,
        attempts: u32,
    ) -> StepResult {
        StepResult {
            step_index,
            description: description.to_string(),
            status,
            summary: sub_result.summary.clone(),
            changed_files: sub_result.changed_files.clone(),
            error_message: sub_result.error_message.clone(),
            attempts,
        }
    }

    /// Build a failed [`StepResult`] from a spawn error.
    fn build_failed_step_result(
        &self,
        step_index: usize,
        description: &str,
        error: &ExecutionError,
        attempts: u32,
    ) -> StepResult {
        StepResult {
            step_index,
            description: description.to_string(),
            status: ExecutionStepStatus::Failed,
            summary: String::new(),
            changed_files: Vec::new(),
            error_message: Some(error.to_string()),
            attempts,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Run hooks for a completed step if a hook runner is configured.
    async fn run_hooks_for_step(&self, step_result: &StepResult, events: &mut Vec<ExecutionEvent>) {
        if let Some(ref runner) = self.hook_runner {
            if !runner.should_run(step_result) {
                return;
            }
            match runner.run_hooks(step_result).await {
                crate::hook::HookResult::Continue => {
                    Self::emit_event(
                        events,
                        ExecutionEvent::HookExecuted {
                            step_index: step_result.step_index,
                        },
                    );
                }
                crate::hook::HookResult::Block(reason) => {
                    Self::emit_event(
                        events,
                        ExecutionEvent::HookFailed {
                            step_index: step_result.step_index,
                            error_message: reason,
                        },
                    );
                }
            }
        }
    }

    /// Apply a status transition to the given step in plan state.
    /// After a successful transition, notifies the progress listener.
    async fn mark_step_status(
        &self,
        step_index: usize,
        status: ExecutionStepStatus,
    ) -> Result<(), ExecutionError> {
        let summary = {
            let mut state = self.plan_state.lock().expect("plan state lock poisoned");

            if matches!(status, ExecutionStepStatus::InProgress) {
                state.current_step = Some(step_index);
            }

            state.apply_transition(step_index, status).map_err(|e| {
                ExecutionError::InvalidResult {
                    message: format!("state transition failed: {e}"),
                }
            })?;

            state.progress_summary()
        };

        self.notifier.on_progress_changed(&summary).await;

        Ok(())
    }

    /// Emit an execution event — logs it and appends to the events list.
    fn emit_event(events: &mut Vec<ExecutionEvent>, event: ExecutionEvent) {
        tracing::info!(?event, "execution event");
        events.push(event);
    }
}
