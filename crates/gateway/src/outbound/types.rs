//! Shared outbound dispatch types.

use closeclaw_llm::types::{ContentBlock, UnifiedResponse, UnifiedUsage};

/// Result of a streaming outbound dispatch.
///
/// Carries the accumulated content blocks (for downstream consumers like
/// `append_response`) and the final token usage reported by the LLM's
/// `MessageEnd` event.
#[derive(Debug, Clone)]
pub struct StreamResult {
    /// All [`ContentBlock`]s produced by the renderer during the stream.
    pub content_blocks: Vec<ContentBlock>,
    /// Token usage statistics from the LLM's `MessageEnd` event.
    pub usage: UnifiedUsage,
    /// Number of retry attempts made before the LLM call succeeded.
    pub retry_attempts: u32,
    /// DSL instruction result extracted from processor chain metadata.
    pub dsl_result: Option<String>,
    /// Actually-used (provider/model) call target of this round,
    /// returned by the underlying [`LlmCaller`](closeclaw_common::LlmCaller) invocation via its
    /// `call_with_target` / `call_streaming_with_target` extension
    /// methods. `None` when the caller does not report a target
    /// (test fakes / stub paths) — consumers degrade to the session
    /// config's model semantics.
    pub call_target: Option<closeclaw_common::CallTarget>,
}

impl From<UnifiedResponse> for StreamResult {
    /// Convert a non-streaming `UnifiedResponse` into a `StreamResult`.
    ///
    /// Used by the post-LLM completion path (`finish_llm` /
    /// `clear_busy_and_send`) so both streaming and non-streaming
    /// call sites can share the same downstream handling. `finish_reason`
    /// is dropped because `StreamResult` does not carry one.
    fn from(response: UnifiedResponse) -> Self {
        StreamResult {
            content_blocks: response.content_blocks,
            usage: response.usage,
            retry_attempts: response.retry_attempts,
            dsl_result: None,
            // The `From` conversion has no access to the underlying
            // caller; production call sites attach the actual target
            // from the invocation's returned value.
            call_target: None,
        }
    }
}

impl From<StreamResult> for UnifiedResponse {
    /// Convert a `StreamResult` back into a `UnifiedResponse` for
    /// `ChatSession::append_response`, which only accepts the legacy
    /// shape. `finish_reason` is set to `None` because streaming does
    /// not surface a structured finish reason.
    fn from(result: StreamResult) -> Self {
        UnifiedResponse {
            content_blocks: result.content_blocks,
            usage: result.usage,
            finish_reason: None,
            retry_attempts: result.retry_attempts,
        }
    }
}

/// Bundled context for streaming outbound dispatch.
///
/// Groups trace metadata and optional session-assembled content that
/// every streaming outbound helper needs, keeping individual parameter
/// lists within the project's 6-parameter hard limit.
#[derive(Debug, Clone, Default)]
pub struct OutboundMeta {
    /// Inbound trace ID for debug-log event correlation.
    pub trace_id: Option<String>,
    /// Inbound session key for debug-log event correlation.
    pub session_key: Option<String>,
    /// Root span ID for debug-log child span derivation.
    pub span_id: Option<String>,
    /// Session-assembled content blocks (used by `send_outbound_streaming_assembled`).
    pub session_content_blocks: Vec<ContentBlock>,
    /// Session-assembled usage override.
    pub session_usage: Option<UnifiedUsage>,
}

/// Outcome of a single outbound dispatch.
///
/// Distinguishes between "message delivered" and "delivery failed but user
/// notified" so callers like the drain loop can keep `delivered` counts honest
/// without double-notifying or retrying.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendOutcome {
    /// Original message delivered successfully.
    Sent,
    /// Original message delivery failed; a failure notification was sent to
    /// the user via the simplified path. The message was NOT delivered.
    Notified,
}

/// Owned debug-log identity pair passed to
/// [`Gateway::send_outbound`](crate::Gateway::send_outbound).
///
/// Groups the inbound trace metadata (`trace_id` / `session_key`) used for
/// debug-log event correlation, keeping the method's parameter list within
/// the project's 6-parameter hard limit (same pattern as [`OutboundMeta`],
/// but owned: the values are moved into the dispatch context below).
///
/// `Default` yields `None` for both fields — the value used by nearly all
/// call sites.
#[derive(Debug, Clone, Default)]
pub struct SendOutboundIds {
    /// Inbound trace ID for debug-log event correlation.
    pub trace_id: Option<String>,
    /// Inbound session key for debug-log event correlation.
    pub session_key: Option<String>,
}

/// Thread-routing fields resolved from the session checkpoint for an
/// outbound dispatch.
///
/// Shared by the batch and streaming entries (resolved via
/// `Gateway::resolve_outbound_thread_routing` in
/// [`chain_prep`](super::chain_prep)); both dispatch contexts forward the
/// pair to the IM plugin so replies land in the originating thread.
#[derive(Debug, Clone, Default)]
pub(crate) struct OutboundThreadRouting {
    /// Optional thread/topic ID for directing the message into a thread.
    pub thread_id: Option<String>,
    /// Outbound directed reference for 定向回复.
    pub reply_ref: Option<String>,
}
