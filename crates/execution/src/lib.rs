//! Plan execution engine crate.
//!
//! Provides core scheduling, execution mode strategies,
//! and sub-agent result parsing for the Plan execution pipeline.

pub mod engine;
pub mod error;
pub mod event;
pub mod execution_state;
pub mod execution_types;
pub mod hook;
pub mod mode;
pub mod notification;
pub mod spawn;
pub mod types;

pub use engine::{ExecutionEngine, ExecutionReport, StepResult};

pub use error::ExecutionError;
pub use event::ExecutionEvent;
pub use hook::{
    CustomHook, HookError, HookResult, HookRunner, NotifyHook, StepHook, VerificationHook,
};
pub use mode::{ExecutionStrategy, InlineMode, SpawnAllStepsMode, SpawnPerStepMode};
pub use notification::{parse_subagent_result, ParseError};
pub use execution_state::{
    apply_transition, current_step_index, get_step_status, init_execution_steps,
    progress_summary, validate_transition, DefaultPlanStateWriter, ExecutionState,
    PlanStateWriter, step_status_to_marker,
};
pub use execution_types::{ExecutionStep, ExecutionStepStatus, TransitionError};
pub use spawn::SpawnAdapter;
pub use types::{ExecutionConfig, ExecutionMode, SubAgentResult, VerifyTrigger};

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod engine_notifier_tests;

#[cfg(test)]
mod engine_status_tests;

#[cfg(test)]
mod hook_tests;

#[cfg(test)]
mod types_tests;

#[cfg(test)]
mod permission_tests;

#[cfg(test)]
mod engine_step_selection_tests;

#[cfg(test)]
mod execution_state_tests;
