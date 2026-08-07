use uuid::Uuid;

/// Trace context for correlating log events across a message lifecycle.
///
/// Each message chain gets a root `TraceContext` with a unique `trace_id`.
/// Child spans derive from the parent span to track sub-calls (LLM invocations,
/// tool execution, agent spawning, etc.).
#[derive(Debug, Clone)]
pub struct TraceContext {
    /// Message chain unique identifier (generated at webhook arrival or by module).
    pub trace_id: String,
    /// Current span identifier.
    pub span_id: String,
    /// Parent span identifier (populated for child spans, empty for root).
    pub parent_span_id: String,
}

impl TraceContext {
    /// Create a root span for a new trace.
    pub fn new_root(trace_id: String) -> Self {
        Self {
            trace_id,
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: String::new(),
        }
    }

    /// Derive a child span from the current context.
    ///
    /// The child inherits `trace_id` and `session_key` from the parent,
    /// and sets `parent_span_id` to the current `span_id`.
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: self.span_id.clone(),
        }
    }
}

#[cfg(test)]
#[path = "trace_context_tests.rs"]
mod trace_context_tests;
