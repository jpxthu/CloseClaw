//! Plan execution engine crate.
//!
//! Provides core scheduling, retry logic, execution mode strategies,
//! and sub-agent result parsing for the Plan execution pipeline.

pub mod error;
pub mod event;
pub mod mode;
pub mod notification;
pub mod spawn;
pub mod types;

pub use error::ExecutionError;
pub use event::ExecutionEvent;
pub use mode::{ExecutionStrategy, InlineMode, SpawnAllStepsMode, SpawnPerStepMode};
pub use notification::{parse_subagent_result, ParseError};
pub use spawn::SpawnAdapter;
pub use types::{ExecutionConfig, ExecutionMode, RetryStrategy, SubAgentResult, VerifyTrigger};
