//! Types for session run health detection and failure classification.
//!
//! These types are consumed only by the session crate and do not
//! meet the common-crate admission threshold, so they live here.

/// Overall health status of a session turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    /// The session turn completed successfully.
    Healthy,
    /// The session turn is unhealthy for the given reason.
    Unhealthy(FailureCategory),
}

/// Classification of why a session turn is unhealthy.
///
/// Each category maps to a distinct recovery strategy in the
/// unhealthy handler layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureCategory {
    /// Transient error that may succeed on retry (e.g. API timeout,
    /// network blip). Retried with exponential backoff; upgraded to
    /// `Unrecoverable` after exhausting retries.
    Retryable,
    /// LLM produced an empty, structure-only, or otherwise invalid
    /// response. Retried with an injected retry instruction; escalated
    /// to user notification after exhausting retries.
    InvalidResponse,
    /// Non-recoverable error (auth failure, missing model, context
    /// exhaustion). No retry attempted; user is notified immediately.
    Unrecoverable,
    /// Side effects have already occurred (tool executed, message sent)
    /// but the LLM response was interrupted. User is asked to verify;
    /// no automatic rollback.
    SideEffectOccurred,
}

/// A specific hard-rule violation detected during health check.
///
/// Each variant carries context information useful for diagnostics
/// and logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardRuleViolation {
    /// Turn elapsed time exceeded the configured threshold.
    Timeout {
        /// Milliseconds elapsed since turn start.
        elapsed_ms: u64,
        /// Configured timeout threshold in milliseconds.
        threshold_ms: u64,
    },
    /// LLM returned no usable content (no text, no tool calls, no
    /// thinking output).
    EmptyResponse,
    /// LLM produced only thinking/reasoning output with no
    /// text or tool calls.
    ThinkingOnlyResponse,
    /// Response structure is malformed or missing required fields.
    StructuralAnomaly {
        /// Human-readable description of what was wrong.
        detail: String,
    },
    /// Retry counter has reached or exceeded the configured maximum.
    RetryExhausted {
        /// Number of retries attempted.
        attempts: u32,
        /// Maximum allowed retries.
        max_retries: u32,
    },
    /// Side effects (tool calls) were executed during this turn, but
    /// the LLM response was interrupted (no text, no tool calls).
    SideEffectOccurred,
}

/// Snapshot of session state at turn boundary, used as input to the
/// health-check pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheckInput {
    /// Whether the LLM response contained any text content.
    pub has_text: bool,
    /// Whether the LLM response contained any tool call.
    pub has_tool_calls: bool,
    /// Whether the LLM response contained thinking/reasoning output.
    pub has_thinking: bool,
    /// Number of consecutive retries on the current turn.
    pub retry_count: u32,
    /// Wall-clock duration of the turn in milliseconds.
    pub turn_duration_ms: u64,
    /// Whether the LLM response parsed successfully (required fields
    /// present, schema valid).
    pub is_structurally_valid: bool,
    /// Optional detail describing a structural anomaly, if any.
    pub structural_anomaly_detail: Option<String>,
    /// Whether tool calls were executed during this turn (side effects
    /// occurred). This is set by the gateway layer based on the presence
    /// of `ToolUse` content blocks in the LLM response.
    pub side_effect_occurred: bool,
}

/// Result produced by the health-check pipeline for a single turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthCheckOutput {
    /// Aggregate health status after all hard rules have been evaluated.
    pub status: HealthStatus,
    /// All hard-rule violations that were detected (may be empty if
    /// healthy, or contain multiple violations).
    pub violations: Vec<HardRuleViolation>,
    /// The suggested failure category derived from violations, if any.
    /// The unhealthy handler uses this to determine the recovery
    /// strategy.
    pub suggested_category: Option<FailureCategory>,
}

/// Retry / backoff policy parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts before escalation.
    pub max_retries: u32,
    /// Initial delay in milliseconds before the first retry.
    pub initial_delay_ms: u64,
    /// Multiplier applied to the delay after each consecutive retry
    /// (e.g. 2.0 = standard exponential backoff).
    pub backoff_multiplier: f64,
}

/// Mapping from a hard-rule violation to a failure category.
///
/// This is the canonical mapping used by both the hard-rule engine
/// and the unhealthy handler.
impl From<&HardRuleViolation> for FailureCategory {
    fn from(violation: &HardRuleViolation) -> Self {
        match violation {
            HardRuleViolation::Timeout { .. } => FailureCategory::Retryable,
            HardRuleViolation::EmptyResponse => FailureCategory::InvalidResponse,
            HardRuleViolation::ThinkingOnlyResponse => FailureCategory::InvalidResponse,
            HardRuleViolation::StructuralAnomaly { .. } => FailureCategory::InvalidResponse,
            HardRuleViolation::RetryExhausted { .. } => FailureCategory::Unrecoverable,
            HardRuleViolation::SideEffectOccurred => FailureCategory::SideEffectOccurred,
        }
    }
}

/// Context passed to the hook reviewer for turn-level quality gate
/// evaluation.  Carries the same turn snapshot data that the
/// hard-rule engine sees, plus recent tool-call history needed by
/// `LoopCheck` and `ProgressCheck`.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// The assistant's text output for this turn.
    pub text: String,
    /// Tool calls made in this turn.
    pub tool_calls: Vec<HookToolCallInfo>,
    /// Tool results returned in this turn.
    pub tool_results: Vec<String>,
    /// Tool calls from the last N turns (for loop detection).
    pub recent_tool_calls: Vec<HookToolCallInfo>,
}

/// Summary of a single tool call for hook review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookToolCallInfo {
    /// Name of the tool being called.
    pub name: String,
    /// Serialized input/arguments.
    pub input: String,
}

/// Action the unhealthy handler prescribes in response to a failure
/// category and current backoff state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoverableAction {
    /// Retry after a delay, optionally injecting an instruction for
    /// the LLM.
    Retry {
        /// Delay in milliseconds before the next attempt.
        delay_ms: u64,
        /// Optional instruction to inject into the next LLM call.
        instruction: Option<String>,
    },
    /// Notify the user with a message and stop the turn.
    NotifyUser {
        /// Message to display to the user.
        message: String,
    },
    /// Stop the session turn without user notification (e.g. after
    /// side effects have been reported and user is verifying).
    Stop {
        /// Reason for stopping.
        reason: String,
    },
}
