//! Plan execution engine crate.
//!
//! Provides core scheduling, retry logic, execution mode strategies,
//! and sub-agent result parsing for the Plan execution pipeline.

pub mod engine;
pub mod error;
pub mod event;
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
pub use spawn::SpawnAdapter;
pub use types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod hook_tests;

#[cfg(test)]
mod types_tests;

#[cfg(test)]
mod permission_tests;
