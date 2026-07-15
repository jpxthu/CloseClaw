//! Per-request context for dynamic-layer system prompt injection.
//!
//! Stores inbound message metadata needed by
//! [`build_dynamic_sections`]
//! ([closeclaw_system_prompt::inject::build_dynamic_sections])
//! at LLM request time. The session crate holds this type via
//! [`ConversationSession::set_request_context`] so that the
//! gateway layer can pass inbound metadata without a reverse
//! dependency.
//!
//! This type lives in `closeclaw-common` (Layer 0) to avoid the
//! session → gateway dependency that
//! [`MessageMetadata`]
//! ([closeclaw_gateway::session_handler::MessageMetadata])
//! would require.

use serde::{Deserialize, Serialize};

/// Metadata about the current inbound message, carried into the
/// session for dynamic-layer construction.
///
/// Set by the gateway before each [`invoke_llm`]
/// ([closeclaw_session::llm_session::ConversationSession::invoke_llm])
/// call so the session can build fresh dynamic sections
/// (ChannelContext, etc.) with up-to-date timestamps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestContext {
    /// Open ID of the message sender.
    pub sender_id: String,
    /// Channel identifier (e.g. "feishu", "telegram").
    pub channel: String,
    /// Unix timestamp (seconds) when the message was created.
    pub timestamp: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_context_serialization_roundtrip() {
        let ctx = RequestContext {
            sender_id: "ou_ser".to_string(),
            channel: "slack".to_string(),
            timestamp: 42,
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let restored: RequestContext = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.sender_id, "ou_ser");
        assert_eq!(restored.channel, "slack");
        assert_eq!(restored.timestamp, 42);
    }
}
