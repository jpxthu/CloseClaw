//! Execution mode strategies — three strategies for dispatching steps.

use async_trait::async_trait;

use crate::error::ExecutionError;
use crate::spawn::SpawnAdapter;
use crate::types::SubAgentResult;
use closeclaw_common::ExecutionStepStatus;

/// Strategy trait for executing a single step in a given mode.
///
/// Each execution mode (inline, spawn per step, spawn all steps)
/// implements this trait to define its dispatch behavior.
#[async_trait]
pub trait ExecutionStrategy: Send + Sync {
    /// Execute a single step and return its structured result.
    async fn execute_step(
        &self,
        step_index: usize,
        task: &str,
        context: &str,
    ) -> Result<SubAgentResult, ExecutionError>;
}

/// Inline execution mode — steps are executed in the parent session.
///
/// The parent session acts as the executor; the result is a placeholder
/// indicating the step is in progress, with the task description as
/// the summary. The actual work is done by the caller's LLM loop.
pub struct InlineMode;

#[async_trait]
impl ExecutionStrategy for InlineMode {
    async fn execute_step(
        &self,
        step_index: usize,
        task: &str,
        _context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        tracing::info!(step_index, "inline mode: returning step for LLM execution");
        Ok(SubAgentResult {
            step_index,
            status: ExecutionStepStatus::InProgress,
            summary: task.to_string(),
            changed_files: Vec::new(),
            error_message: None,
        })
    }
}

/// Spawn-per-step execution mode — each step spawns an independent sub-agent.
///
/// The sub-agent runs with a clean context for each step, keeping
/// failures isolated.
pub struct SpawnPerStepMode<'a> {
    /// The spawn adapter to use for creating sub-agents.
    pub adapter: &'a dyn SpawnAdapter,
}

#[async_trait]
impl ExecutionStrategy for SpawnPerStepMode<'_> {
    async fn execute_step(
        &self,
        step_index: usize,
        task: &str,
        context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        tracing::info!(step_index, "spawning sub-agent for single step");
        self.adapter.spawn_run(task, context).await
    }
}

/// Spawn-all-steps execution mode — a single sub-agent executes all steps.
///
/// One sub-agent receives the full task list and context, executing all
/// steps in sequence within a single session.
pub struct SpawnAllStepsMode<'a> {
    /// The spawn adapter to use for creating the sub-agent.
    pub adapter: &'a dyn SpawnAdapter,
}

#[async_trait]
impl ExecutionStrategy for SpawnAllStepsMode<'_> {
    async fn execute_step(
        &self,
        step_index: usize,
        task: &str,
        context: &str,
    ) -> Result<SubAgentResult, ExecutionError> {
        tracing::info!(step_index, "spawning sub-agent for all steps");
        self.adapter.spawn_run(task, context).await
    }
}
