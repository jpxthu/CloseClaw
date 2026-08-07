//! Debug log framework for CloseClaw.
//!
//! Provides structured JSONL logging with trace context propagation,
//! level filtering, credential redaction, and daily log rotation.

mod config;
mod event;
mod level;
mod level_filter;
mod redaction;
mod retention;
mod trace_context;
mod writer;

pub use config::{DebugLogConfig, DebugLogConfigError};
pub use event::LogEvent;
pub use level::LogLevel;
pub use level_filter::LevelFilter;
pub use redaction::{PatternMatch, RedactionEngine, RedactionPattern};
pub use retention::{LogRetention, LogRetentionError};
pub use trace_context::TraceContext;
pub use writer::{LogWriter, LogWriterError};
