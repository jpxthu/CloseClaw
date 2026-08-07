use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{LogLevel, TraceContext};

/// A structured log event for the debug log framework.
///
/// Events are serialized to JSONL (one JSON object per line) for storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    /// Message chain unique identifier.
    pub trace_id: String,
    /// Current span identifier.
    pub span_id: String,
    /// Parent span identifier (empty for root spans).
    pub parent_span_id: String,
    /// Optional session key for message chain correlation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    /// Event timestamp in milliseconds (UTC).
    pub timestamp: DateTime<Utc>,
    /// Event severity level.
    pub level: LogLevel,
    /// Source module that produced this event (e.g. "gateway", "session").
    pub source_module: String,
    /// Event type for categorization (e.g. "message.arrived", "llm.call.start").
    pub event_type: String,
    /// Structured payload as arbitrary JSON.
    pub payload: serde_json::Value,
}

impl LogEvent {
    /// Create a new log event from a trace context and metadata.
    pub fn new(
        ctx: &TraceContext,
        session_key: Option<String>,
        level: LogLevel,
        source_module: impl Into<String>,
        event_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            trace_id: ctx.trace_id.clone(),
            span_id: ctx.span_id.clone(),
            parent_span_id: ctx.parent_span_id.clone(),
            session_key,
            timestamp: Utc::now(),
            level,
            source_module: source_module.into(),
            event_type: event_type.into(),
            payload,
        }
    }

    /// Serialize this event to a single JSON line (JSONL format).
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize a single JSONL line back into a LogEvent.
    pub fn from_jsonl(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }
}
