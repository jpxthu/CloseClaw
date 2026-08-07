//! Debug log framework for CloseClaw.
//!
//! Provides structured JSONL logging with trace context propagation,
//! level filtering, credential redaction, and daily log rotation.

mod event;
mod level;
mod level_filter;
mod redaction;
mod trace_context;

pub use event::LogEvent;
pub use level::LogLevel;
pub use level_filter::LevelFilter;
pub use redaction::{PatternMatch, RedactionEngine, RedactionPattern};
pub use trace_context::TraceContext;
