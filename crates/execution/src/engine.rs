//! Core execution engine — orchestrates step-by-step execution with
//! retry logic and state management. Retries are controlled by
//! [`ExecutionConfig`].

use std::sync::{Arc, Mutex};

use closeclaw_common::{
    ExecutionPermissionCheck, ExecutionStepStatus, PlanState, PlanStateNotifier, PlanStatus,
};

use crate::error::ExecutionError;
use crate::event::ExecutionEvent;
use crate::hook::HookRunner;
use crate::spawn::SpawnAdapter;
use crate::types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult};

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
    /// If a hook returned Block, the reason is recorded here.
    pub hook_blocked: Option<String>,
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
    /// Whether any hook returned Block during execution.
    pub hook_blocked: bool,
    /// Events emitted during execution.
    pub events: Vec<ExecutionEvent>,
}

/// Core execution engine — drives the step-by-step scheduling loop.
/// Generic over `S: SpawnAdapter` so the dispatch mechanism can be swapped.
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
    /// Optional permission checker — called before each step dispatch.
    permission: Option<Arc<dyn ExecutionPermissionCheck>>,
}
// --- Public interface ---
impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Create a new execution engine.
    ///
    /// When no hook runner is provided via [`with_hook_runner`], a default
    /// [`HookRunner`] is constructed using `config.verify_trigger`.
    pub fn new(
        plan_state: Arc<Mutex<PlanState>>,
        config: ExecutionConfig,
        adapter: S,
        notifier: Arc<dyn PlanStateNotifier>,
        permission: Option<Arc<dyn ExecutionPermissionCheck>>,
    ) -> Self {
        let hook_runner = Some(Self::build_default_hook_runner(&config));
        Self {
            plan_state,
            config,
            adapter,
            notifier,
            hook_runner,
            permission,
        }
    }

    /// Create a new execution engine with a hook runner.
    pub fn with_hook_runner(
        plan_state: Arc<Mutex<PlanState>>,
        config: ExecutionConfig,
        adapter: S,
        notifier: Arc<dyn PlanStateNotifier>,
        hook_runner: HookRunner,
        permission: Option<Arc<dyn ExecutionPermissionCheck>>,
    ) -> Self {
        Self {
            plan_state,
            config,
            adapter,
            notifier,
            hook_runner: Some(hook_runner),
            permission,
        }
    }

    /// Build context string for continue-mode retries.
    ///
    /// In continue mode, the previous error is injected so the
    /// sub-agent can see error history and adapt.
    fn continue_error_context(error_history: &[String]) -> String {
        if error_history.is_empty() {
            String::new()
        } else {
            format!(
                "[continue retry] Previous errors:\n{}",
                error_history.join("\n")
            )
        }
    }

    /// Execute all provided steps sequentially and return a report.
    ///
    /// When `config.step_selection` is `Some`, only the steps at the
    /// specified indices are executed. Indices are validated against the
    /// provided step list; invalid indices are rejected with
    /// [`ExecutionError::InvalidStepSelection`].
    pub async fn execute(&self, steps: &[String]) -> Result<ExecutionReport, ExecutionError> {
        let filtered = self.filter_steps(steps)?;

        {
            let mut state = self.plan_state.lock().expect("plan state lock poisoned");
            state.init_execution_steps(filtered.clone());
        }

        match self.config.mode {
            ExecutionMode::SpawnAllSteps => self.execute_spawn_all(&filtered).await,
            ExecutionMode::SpawnPerStep | ExecutionMode::Inline => {
                self.execute_step_by_step(&filtered).await
            }
        }
    }

    /// Filter steps based on `step_selection` config.
    fn filter_steps(&self, steps: &[String]) -> Result<Vec<String>, ExecutionError> {
        match &self.config.step_selection {
            Some(indices) if indices.is_empty() => Ok(Vec::new()),
            Some(indices) => {
                let mut selected = Vec::with_capacity(indices.len());
                for &idx in indices {
                    if idx >= steps.len() {
                        return Err(ExecutionError::InvalidStepSelection {
                            index: idx,
                            total: steps.len(),
                        });
                    }
                    selected.push(steps[idx].clone());
                }
                Ok(selected)
            }
            None => Ok(steps.to_vec()),
        }
    }

    /// Access a snapshot of the current plan state.
    pub fn plan_state_snapshot(&self) -> PlanState {
        self.plan_state
            .lock()
            .expect("plan state lock poisoned")
            .clone()
    }

    /// Build a default [`HookRunner`] from the given config.
    ///
    /// The runner uses `config.verify_trigger` but has no hooks registered.
    fn build_default_hook_runner(config: &ExecutionConfig) -> HookRunner {
        HookRunner::new(config.verify_trigger)
    }

    /// Borrow the hook runner, if one is configured.
    pub fn hook_runner_ref(&self) -> Option<&HookRunner> {
        self.hook_runner.as_ref()
    }

    /// Borrow the spawn adapter (useful for test assertions).
    pub fn adapter_ref(&self) -> &S {
        &self.adapter
    }
}

// --- Step-by-step execution (SpawnPerStep / Inline) ---
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
            let is_hook_blocked = step_result.hook_blocked.is_some();
            results.push(step_result);

            if is_failed {
                failed_step = Some(i);
                break;
            }
            if is_hook_blocked {
                tracing::info!(step_index = i, "hook blocked — stopping execution");
                break;
            }
        }

        let all_completed = failed_step.is_none() && results.len() == steps_owned.len();
        let hook_blocked = results.iter().any(|r| r.hook_blocked.is_some());

        if all_completed {
            tracing::info!("all {} steps completed successfully", steps_owned.len());
            self.transition_plan_to_completed();
            events.push(ExecutionEvent::AllCompleted);
            self.notifier.on_plan_completed().await;
        } else if let Some(idx) = failed_step {
            tracing::warn!("execution stopped at step {idx}");
        }

        Ok(ExecutionReport {
            steps: results,
            all_completed,
            failed_step,
            hook_blocked,
            events,
        })
    }
}

// --- SpawnAllSteps execution ---
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
        let mut error_history: Vec<String> = Vec::new();

        for i in 0..steps_owned.len() {
            Self::emit_event(&mut events, ExecutionEvent::StepStarted { step_index: i });
        }

        // Check permission for the composite task before first dispatch.
        if let Err(ExecutionError::PermissionDenied { reason, .. }) =
            self.check_permission(0, &task).await
        {
            tracing::warn!(reason, "permission denied — marking all steps as failed");
            let results: Vec<StepResult> = steps_owned
                .iter()
                .enumerate()
                .map(|(i, description)| {
                    Self::emit_event(
                        &mut events,
                        ExecutionEvent::StepFailed {
                            step_index: i,
                            error_message: format!("permission denied: {reason}"),
                        },
                    );
                    StepResult {
                        step_index: i,
                        description: description.clone(),
                        status: ExecutionStepStatus::Failed,
                        summary: String::new(),
                        changed_files: Vec::new(),
                        error_message: Some(format!("permission denied: {reason}")),
                        attempts: 0,
                        hook_blocked: None,
                    }
                })
                .collect();
            return Ok(ExecutionReport {
                steps: results,
                all_completed: false,
                failed_step: Some(0),
                hook_blocked: false,
                events,
            });
        }

        loop {
            attempt += 1;
            self.mark_step_status(0, ExecutionStepStatus::InProgress)
                .await?;

            let context = if self.config.retry_strategy == RetryStrategy::Continue {
                Self::continue_error_context(&error_history)
            } else {
                String::new()
            };

            match self.adapter.spawn_run(&task, &context).await {
                Ok(sub_result) => {
                    return self
                        .handle_spawn_all_success(steps_owned, sub_result, attempt, &mut events)
                        .await;
                }
                Err(e) => {
                    if attempt < max_attempts {
                        error_history.push(e.to_string());
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
        )
        .await;

        let all_completed = failed_step.is_none() && results.len() == steps_owned.len();
        let hook_blocked = results.iter().any(|r| r.hook_blocked.is_some());
        if all_completed {
            self.transition_plan_to_completed();
            events.push(ExecutionEvent::AllCompleted);
            self.notifier.on_plan_completed().await;
        }

        Ok(ExecutionReport {
            steps: results,
            all_completed,
            failed_step,
            hook_blocked,
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
            hook_blocked: None,
        });
        if !step0_failed {
            let block_reason = self
                .run_hooks_for_step(&results[results.len() - 1], events)
                .await;
            results.last_mut().expect("step 0 must exist").hook_blocked = block_reason;
        }
        Ok(failed_step)
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Build StepResult entries for steps 1+ in SpawnAllSteps mode.
    async fn build_spawn_all_remaining_results(
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
                hook_blocked: None,
            });

            if !step0_failed {
                let len = results.len();
                let block_reason = self.run_hooks_for_step(&results[len - 1], events).await;
                results.last_mut().expect("step must exist").hook_blocked = block_reason;
            }
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
                    hook_blocked: None,
                }
            })
            .collect();

        Ok(ExecutionReport {
            steps: results,
            all_completed: false,
            failed_step: Some(0),
            hook_blocked: false,
            events: events.clone(),
        })
    }
}

// --- Single-step retry execution ---
/// Outcome of a non-final dispatch attempt (retryable).
enum RetryableOutcome {
    /// Sub-agent returned a failed status with an optional error message.
    SubAgentFailed(Option<String>),
    /// Spawn itself failed with an error message.
    SpawnError(String),
}

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
        let mut error_history: Vec<String> = Vec::new();

        loop {
            attempt += 1;
            if attempt == 1 {
                Self::emit_event(events, ExecutionEvent::StepStarted { step_index });
            }
            self.mark_step_status(step_index, ExecutionStepStatus::InProgress)
                .await?;
            tracing::info!(step_index, attempt, max_attempts, "dispatching step");

            let context = if self.config.retry_strategy == RetryStrategy::Continue {
                Self::continue_error_context(&error_history)
            } else {
                String::new()
            };

            let (final_result, retryable) = match self
                .dispatch_step_with_context(
                    step_index,
                    description,
                    &context,
                    attempt,
                    max_attempts,
                    events,
                )
                .await
            {
                Ok(tuple) => tuple,
                Err(ExecutionError::PermissionDenied { reason, .. }) => {
                    tracing::warn!(
                        step_index,
                        reason,
                        "permission denied — marking step as failed"
                    );
                    let denial_msg = format!("permission denied: {reason}");
                    Self::emit_event(
                        events,
                        ExecutionEvent::StepFailed {
                            step_index,
                            error_message: denial_msg.clone(),
                        },
                    );
                    return Ok(StepResult {
                        step_index,
                        description: description.to_string(),
                        status: ExecutionStepStatus::Failed,
                        summary: String::new(),
                        changed_files: Vec::new(),
                        error_message: Some(format!("permission denied: {reason}")),
                        attempts: 1,
                        hook_blocked: None,
                    });
                }
                Err(e) => return Err(e),
            };
            if let Some(final_result) = final_result {
                return Ok(final_result);
            }
            // Non-final failure — record error for continue mode.
            if self.config.retry_strategy == RetryStrategy::Continue {
                if let Some(outcome) = retryable {
                    let err_msg = match outcome {
                        RetryableOutcome::SubAgentFailed(msg) => {
                            msg.unwrap_or_else(|| "unknown error".into())
                        }
                        RetryableOutcome::SpawnError(msg) => msg,
                    };
                    error_history.push(err_msg);
                }
            }
        }
    }
}

impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Check execution permission for a step before dispatch.
    ///
    /// Returns `Ok(())` if no permission checker is configured or if the check
    /// passes. Returns `Err(PermissionDenied { step_index, reason })` if the
    /// check fails.
    async fn check_permission(
        &self,
        step_index: usize,
        description: &str,
    ) -> Result<(), ExecutionError> {
        if let Some(ref checker) = self.permission {
            checker
                .check_execution(description)
                .await
                .map_err(|denied| ExecutionError::PermissionDenied {
                    step_index,
                    reason: denied.reason,
                })
        } else {
            Ok(())
        }
    }

    /// Dispatch a single step attempt and process the result.
    ///
    /// `context` is passed through to [`SpawnAdapter::spawn_run`],
    /// allowing retry strategies to inject error history.
    ///
    /// Returns `(final_result, retryable_outcome)`. If the step
    /// succeeded or retries are exhausted, `final_result` is `Some`.
    /// Otherwise `retryable_outcome` carries the error for continue mode.
    async fn dispatch_step_with_context(
        &self,
        step_index: usize,
        description: &str,
        context: &str,
        attempt: u32,
        max_attempts: u32,
        events: &mut Vec<ExecutionEvent>,
    ) -> Result<(Option<StepResult>, Option<RetryableOutcome>), ExecutionError> {
        // Check permission before dispatching — permission denial is not retryable.
        self.check_permission(step_index, description).await?;

        match self.adapter.spawn_run(description, context).await {
            Ok(sub_result) => {
                let final_result = self
                    .handle_step_spawn_result(
                        step_index,
                        description,
                        sub_result.clone(),
                        attempt,
                        max_attempts,
                        events,
                    )
                    .await?;
                let retryable = if final_result.is_none() {
                    Some(RetryableOutcome::SubAgentFailed(sub_result.error_message))
                } else {
                    None
                };
                Ok((final_result, retryable))
            }
            Err(e) => {
                let final_result = self
                    .handle_step_spawn_error(
                        step_index,
                        description,
                        e.clone(),
                        attempt,
                        max_attempts,
                        events,
                    )
                    .await?;
                let retryable = if final_result.is_none() {
                    Some(RetryableOutcome::SpawnError(e.to_string()))
                } else {
                    None
                };
                Ok((final_result, retryable))
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

        let mut step_result = self.build_step_result(
            step_index,
            description,
            sub_result.status,
            sub_result,
            attempt,
        );

        let block_reason = self.run_hooks_for_step(&step_result, events).await;
        step_result.hook_blocked = block_reason;

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
            hook_blocked: None,
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
            hook_blocked: None,
        }
    }
}

// --- Helpers ---
impl<S: SpawnAdapter> ExecutionEngine<S> {
    /// Run hooks for a completed step if a hook runner is configured.
    ///
    /// Returns `Some(reason)` if a hook blocked, or `None` if all hooks
    /// passed (or no runner is configured).
    async fn run_hooks_for_step(
        &self,
        step_result: &StepResult,
        events: &mut Vec<ExecutionEvent>,
    ) -> Option<String> {
        if let Some(ref runner) = self.hook_runner {
            if !runner.should_run(step_result) {
                return None;
            }
            match runner.run_hooks(step_result).await {
                crate::hook::HookResult::Continue => {
                    Self::emit_event(
                        events,
                        ExecutionEvent::HookExecuted {
                            step_index: step_result.step_index,
                        },
                    );
                    None
                }
                crate::hook::HookResult::Block(reason) => {
                    Self::emit_event(
                        events,
                        ExecutionEvent::HookFailed {
                            step_index: step_result.step_index,
                            error_message: reason.clone(),
                        },
                    );
                    Some(reason)
                }
            }
        } else {
            None
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

    /// Transition the plan state status to Completed.
    ///
    /// Called when all steps complete successfully. If the transition fails
    /// (e.g. status is already Completed or not in Executing), a warning is
    /// logged but execution continues normally.
    fn transition_plan_to_completed(&self) {
        let mut state = self.plan_state.lock().expect("plan state lock poisoned");
        if let Err(e) = state.transition_status(PlanStatus::Completed) {
            tracing::warn!(
                current_status = ?state.status,
                error = %e,
                "failed to transition plan status to Completed"
            );
        }
    }

    /// Emit an execution event — logs it and appends to the events list.
    fn emit_event(events: &mut Vec<ExecutionEvent>, event: ExecutionEvent) {
        tracing::info!(?event, "execution event");
        events.push(event);
    }
}
