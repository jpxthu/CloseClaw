//! LLM Provider abstraction — pure configuration and HTTP send interface.
//!
//! This module defines the [`Provider`] trait, which is the **sole interface**
//! through which the LLM framework interacts with a concrete provider
//! implementation (OpenAI, Anthropic, GLM, DeepSeek, etc.).
//!
//! A `Provider` is responsible only for **carrying configuration** (URL, credentials,
//! HTTP client) and for **performing the actual HTTP request/response cycle**.
//! All request building (`build_request`) and response parsing (`parse_response`,
//! `parse_sse`) are handled by a [`ChatProtocol`][crate::llm::ChatProtocol]
//! implementation, which is selected based on the `ProtocolId` returned by
//! `supported_protocols()`.

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use tokio::sync::mpsc;

use crate::llm::types::{InternalRequest, InternalResponse, ProtocolId, RawSseChunk};

/// Errors that can occur during provider-level HTTP operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// HTTP request failed (network error, TLS error, timeout, etc.).
    #[error("HTTP request failed: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Error from a legacy provider adapter.
    #[error("Legacy provider error: {0}")]
    Legacy(String),
}

/// Result type alias for provider operations.
pub type Result<T> = std::result::Result<T, ProviderError>;

/// SSE stream — a channel that delivers raw SSE chunks to the caller.
///
/// The channel is owned by the caller; the provider implementation sends
/// [`RawSseChunk`] values into it until the response is fully consumed or
/// an error occurs, at which point the channel is closed.
pub type SseStream = mpsc::Receiver<RawSseChunk>;

/// LLM provider trait — configuration + HTTP send.
///
/// Implementors hold the credentials, base URL, and HTTP client for
/// a specific LLM API provider.  The trait is intentionally narrow: it
/// does **not** know about model lists, retries, or fallback strategies.
///
/// # Design contract
///
/// - All configuration accessors (`id`, `base_url`, `api_key`, …) are **synchronous**
///   because they only return values stored in `Self`.
/// - `send` and `send_streaming` are **asynchronous** because they perform I/O.
/// - `supported_protocols` returns the set of protocol IDs this provider can serve.
///   The framework selects the matching [`ChatProtocol`][crate::llm::ChatProtocol]
///   from the registry and calls `build_request` before invoking `send`.
#[async_trait]
pub trait Provider: Send + Sync {
    // ── Configuration accessors ─────────────────────────────────────────────

    /// Returns the unique identifier for this provider (e.g. `"openai"`, `"anthropic"`).
    fn id(&self) -> &str;

    /// Returns the base URL of the provider's API endpoint
    /// (e.g. `"https://api.openai.com/v1"`).
    fn base_url(&self) -> &str;

    /// Returns the API key used for authentication.
    fn api_key(&self) -> &str;

    /// Returns the set of protocol IDs this provider supports.
    fn supported_protocols(&self) -> &[ProtocolId];

    /// Returns a reference to the underlying HTTP client.
    fn http_client(&self) -> &Client;

    /// Returns additional HTTP headers that should be sent with every request.
    ///
    /// This is additive to the authentication headers already managed internally.
    fn default_headers(&self) -> &HeaderMap;

    // ── Behaviour: HTTP send ─────────────────────────────────────────────────

    /// Sends a structured request to the provider and returns the parsed response.
    ///
    /// The caller is responsible for calling
    /// [`ChatProtocol::build_request`][crate::llm::ChatProtocol::build_request]
    /// first to convert the [`InternalRequest`][crate::llm::InternalRequest] into a
    /// `serde_json::Value` that is suitable for the provider's HTTP endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the HTTP request fails at the network or
    /// protocol layer (TLS, redirect limits, non-success status codes, etc.).
    async fn send(
        &self,
        request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<InternalResponse>;

    /// Sends a streaming request and returns an SSE event stream.
    ///
    /// The caller is responsible for calling
    /// [`ChatProtocol::build_request`][crate::llm::ChatProtocol::build_request]
    /// first, passing `stream: true` in the [`InternalRequest`][crate::llm::InternalRequest].
    ///
    /// The returned [`SseStream`] channel yields one [`RawSseChunk`] per
    /// SSE event received from the wire.  The caller (typically
    /// [`ChatProtocol::parse_sse_stream`][crate::llm::ChatProtocol::parse_sse_stream])
    /// is responsible for converting these chunks into structured
    /// [`StreamEvent`][crate::llm::StreamEvent] values.
    ///
    /// The channel is closed automatically when the response finishes or
    /// when an error occurs.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError`] if the underlying HTTP request cannot be issued.
    /// Note that stream-level errors (malformed SSE, parse errors) are reported
    /// by closing the channel and **not** as a top-level error.
    async fn send_streaming(
        &self,
        request: InternalRequest,
        body: serde_json::Value,
    ) -> Result<SseStream>;
}

#[cfg(test)]
mod tests {
    use crate::llm::types::{InternalMessage, InternalRequest, ProtocolId};

    // ── ProtocolId tests ──────────────────────────────────────────────────────

    #[test]
    fn test_protocol_id_from_str() {
        let id = ProtocolId::from("openai");
        assert_eq!(id.as_str(), "openai");
        assert_eq!(format!("{}", id), "openai");
    }

    #[test]
    fn test_protocol_id_from_string() {
        let id = ProtocolId::from(String::from("anthropic"));
        assert_eq!(id.as_str(), "anthropic");
        assert_eq!(format!("{}", id), "anthropic");
    }

    #[test]
    fn test_protocol_id_display() {
        let id = ProtocolId::new("test-provider");
        assert_eq!(format!("{}", id), "test-provider");
    }

    #[test]
    fn test_protocol_id_clone() {
        let id = ProtocolId::new("clone-me");
        assert_eq!(id.clone(), id);
    }

    #[test]
    fn test_protocol_id_hash() {
        use std::collections::HashSet;
        let id1 = ProtocolId::new("hashed");
        let id2 = ProtocolId::new("hashed");
        let mut set = HashSet::new();
        set.insert(id1);
        set.insert(id2);
        assert_eq!(set.len(), 1);
    }

    // ── InternalRequest serde roundtrip tests ────────────────────────────────

    use crate::session::persistence::ReasoningLevel;

    #[test]
    fn test_internal_request_basic_roundtrip() {
        let req = InternalRequest {
            model: "gpt-4".into(),
            messages: vec![InternalMessage {
                role: "user".into(),
                content: "hello".into(),
            }],
            temperature: 0.7,
            max_tokens: Some(100),
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: InternalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.model, parsed.model);
        assert_eq!(req.messages.len(), parsed.messages.len());
        assert_eq!(req.temperature, parsed.temperature);
        assert_eq!(req.max_tokens, parsed.max_tokens);
        assert_eq!(req.stream, parsed.stream);
    }

    #[test]
    fn test_internal_request_default_temperature_and_stream() {
        let json = r#"{"model":"test","messages":[]}"#;
        let req: InternalRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.temperature, 0.0);
        assert!(!req.stream);
    }

    #[test]
    fn test_internal_request_extra_body_roundtrip() {
        use serde_json::Value;
        let mut extra = serde_json::Map::new();
        extra.insert("top_p".into(), Value::from(0.9));
        extra.insert("presence_penalty".into(), Value::from(0.1));

        let req = InternalRequest {
            model: "test".into(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: true,
            extra_body: extra.clone(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: InternalRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.extra_body.get("top_p"), Some(&Value::from(0.9)));
        assert_eq!(
            parsed.extra_body.get("presence_penalty"),
            Some(&Value::from(0.1))
        );
    }

    #[test]
    fn test_internal_request_empty_extra_body_not_serialized() {
        let req = InternalRequest {
            model: "test".into(),
            messages: vec![],
            temperature: 0.0,
            max_tokens: None,
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("extra_body"));
    }
}
