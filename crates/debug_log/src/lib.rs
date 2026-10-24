//! Debug log framework for CloseClaw.
//!
//! Provides structured JSONL logging with trace context propagation,
//! level filtering, credential redaction, and daily log rotation.

mod config;
mod consumer;
mod debug_log;
mod emit;
mod event;
mod level;
mod level_filter;
mod redaction;
mod retention;
mod test_macros;
mod trace_context;
mod writer;

#[cfg(any(test, feature = "test-util"))]
pub mod test_utils;

pub use config::{DebugLogConfig, DebugLogConfigError};
pub use consumer::{ConsumerError, JsonlLogConsumer, LogConsumer, NormalizedEvent};
pub use debug_log::{DebugLog, DebugLogError};
pub use emit::{emit_event, DebugLogContext, EmitEventParams};
pub use event::LogEvent;
pub use level::LogLevel;
pub use level_filter::LevelFilter;
pub use redaction::{PatternMatch, RedactionEngine, RedactionPattern};
pub use retention::{LogRetention, LogRetentionError};
pub use trace_context::TraceContext;
pub use writer::{LogWriter, LogWriterError};
