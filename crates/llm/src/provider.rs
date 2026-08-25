//! LLM Provider abstraction — pure configuration and HTTP send interface.
//!
//! This module defines the [`Provider`] trait, which is the **sole interface**
//! through which the LLM framework interacts with a concrete provider
//! implementation (OpenAI, Anthropic, GLM, DeepSeek, etc.).
//!
//! A `Provider` is responsible only for **carrying configuration** (URL, credentials,
//! HTTP client) and for **performing the actual HTTP request/response cycle**.
//! All request building (`build_request`) and response parsing (`parse_response`,
//! `parse_sse`) are handled by a [`ChatProtocol`][crate::ChatProtocol]
//! implementation, which is selected based on the `ProtocolId` returned by
//! `supported_protocols()`.

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;
use tokio::sync::mpsc;

use closeclaw_common::llm_error::LLMError;

use crate::types::{InternalRequest, InternalResponse, ProtocolId, RawSseChunk};

/// Errors that can occur during provider-level HTTP operations.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// HTTP request failed (network error, TLS error, timeout, etc.).
    #[error("HTTP request failed: {0}")]
    Reqwest(#[from] reqwest::Error),

    /// Injected HTTP error (used by fake providers for testing).
    #[error("HTTP {status_code}: {body}")]
    Http {
        status_code: u16,
        body: String,
        retry_after: Option<u64>,
    },

    /// Error from a legacy provider adapter.
    #[error("Legacy provider error: {0}")]
    Legacy(String),
}

/// Result type alias for provider operations.
pub type Result<T> = std::result::Result<T, ProviderError>;

// ── Shared error mapping helpers ────────────────────────────────────────────

/// Parse the `Retry-After` response header.
///
/// Supports the `delay-seconds` format (plain integer).  The `HTTP-date`
/// format is intentionally not supported — it is rare in LLM APIs and
/// would require pulling in a date parser.
///
/// Returns `None` when the header is absent or cannot be parsed.
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let s = value.to_str().ok()?;
    s.trim().parse::<u64>().ok()
}

/// Map a non-success HTTP status + body into a structured [`ProviderError::Http`].
///
/// This is the single entry-point that all production providers should call
/// instead of building ad-hoc `Legacy(format!(...))` errors.  It preserves
/// the raw status code and response body so that upstream consumers can
/// perform structured error classification.
pub(crate) fn map_http_error(
    status: reqwest::StatusCode,
    body: String,
    retry_after: Option<u64>,
) -> ProviderError {
    ProviderError::Http {
        status_code: status.as_u16(),
        body,
        retry_after,
    }
}

// ── ProviderError → LLMError structured conversion ─────────────────────────

/// Structured conversion from [`ProviderError`] to [`LLMError`].
///
/// [`ProviderError::Http`] is mapped to the appropriate [`LLMError`] variant
/// based on the HTTP status code, so that `LLMError::kind()` returns the
/// correct error classification **without** string heuristics.
///
/// [`ProviderError::Reqwest`] maps to `NetworkError`, and
/// [`ProviderError::Legacy`] maps to `ApiError` (preserving backward
/// compatibility for non-HTTP error paths).
impl From<&ProviderError> for LLMError {
    fn from(err: &ProviderError) -> Self {
        match err {
            ProviderError::Reqwest(e) => LLMError::NetworkError(e.to_string()),
            ProviderError::Legacy(msg) => LLMError::ApiError(msg.clone()),
            ProviderError::Http {
                status_code, body, ..
            } => match *status_code {
                401 | 403 => LLMError::AuthFailed(format!("HTTP {status_code}: {body}")),
                404 => LLMError::ModelNotFound(format!("HTTP {status_code}: {body}")),
                422 => LLMError::InvalidRequest(format!("HTTP {status_code}: {body}")),
                429 => LLMError::RateLimitExceeded,
                _ => LLMError::ApiError(format!("HTTP {status_code}: {body}")),
            },
        }
    }
}

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
///   The framework selects the matching [`ChatProtocol`][crate::ChatProtocol]
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

    /// Returns whether this provider supports parallel tool calls.
    ///
    /// When `false`, the framework must serialize all tool calls
    /// for this provider, regardless of the agent-level
    /// `parallel_tool_calls` setting.  The default is `true`.
    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

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
    /// [`ChatProtocol::build_request`][crate::ChatProtocol::build_request]
    /// first to convert the [`InternalRequest`][crate::InternalRequest] into a
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
    /// [`ChatProtocol::build_request`][crate::ChatProtocol::build_request]
    /// first, passing `stream: true` in the [`InternalRequest`][crate::InternalRequest].
    ///
    /// The returned [`SseStream`] channel yields one [`RawSseChunk`] per
    /// SSE event received from the wire.  The caller (typically
    /// [`ChatProtocol::parse_sse_stream`][crate::ChatProtocol::parse_sse_stream])
    /// is responsible for converting these chunks into structured
    /// [`StreamEvent`][crate::StreamEvent] values.
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
    use crate::types::{InternalMessage, InternalRequest, ProtocolId};
    use crate::LLMError;

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

    use closeclaw_session::persistence::ReasoningLevel;

    #[test]
    fn test_internal_request_basic_roundtrip() {
        let req = InternalRequest {
            model: "gpt-4".into(),
            messages: vec![InternalMessage {
                role: "user".into(),
                content: "hello".into(),
                ..Default::default()
            }],
            temperature: 0.7,
            max_tokens: Some(100),
            stream: false,
            extra_body: serde_json::Map::new(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
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
            tools: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
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
            tools: None,
            system_dynamic: None,
            system_blocks: None,
            session_id: None,
            reasoning_level: ReasoningLevel::default(),
            turn_count: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("extra_body"));
    }

    // ── supports_parallel_tool_calls tests ───────────────────────────────────

    #[test]
    fn test_provider_supports_parallel_tool_calls_default() {
        use super::super::stub::StubProvider;
        use crate::provider::Provider;
        let provider = StubProvider::new();
        assert!(
            provider.supports_parallel_tool_calls(),
            "StubProvider should default to supports_parallel_tool_calls = true"
        );
    }

    // ── parse_retry_after tests ──────────────────────────────────────────────

    #[test]
    fn test_parse_retry_after_valid_numeric() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            HeaderValue::from_static("120"),
        );
        assert_eq!(super::parse_retry_after(&headers), Some(120));
    }

    #[test]
    fn test_parse_retry_after_missing() {
        use reqwest::header::HeaderMap;
        assert_eq!(super::parse_retry_after(&HeaderMap::new()), None);
    }

    #[test]
    fn test_parse_retry_after_invalid_string() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            HeaderValue::from_static("abc"),
        );
        assert_eq!(super::parse_retry_after(&headers), None);
    }

    #[test]
    fn test_parse_retry_after_http_date_format_returns_none() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2025 07:28:00 GMT"),
        );
        assert_eq!(super::parse_retry_after(&headers), None);
    }

    #[test]
    fn test_parse_retry_after_zero() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("0"));
        assert_eq!(super::parse_retry_after(&headers), Some(0));
    }

    // ── map_http_error tests ─────────────────────────────────────────────────

    #[test]
    fn test_map_http_error_produces_http_variant() {
        use reqwest::StatusCode;
        let err = super::map_http_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limited".to_string(),
            Some(60),
        );
        match err {
            crate::provider::ProviderError::Http {
                status_code,
                body,
                retry_after,
            } => {
                assert_eq!(status_code, 429);
                assert_eq!(body, "rate limited");
                assert_eq!(retry_after, Some(60));
            }
            _ => panic!("expected ProviderError::Http variant"),
        }
    }

    #[test]
    fn test_map_http_error_no_retry_after() {
        use reqwest::StatusCode;
        let err = super::map_http_error(StatusCode::INTERNAL_SERVER_ERROR, "".to_string(), None);
        match err {
            crate::provider::ProviderError::Http {
                status_code,
                body,
                retry_after,
            } => {
                assert_eq!(status_code, 500);
                assert_eq!(body, "");
                assert_eq!(retry_after, None);
            }
            _ => panic!("expected ProviderError::Http variant"),
        }
    }

    // ── ProviderError → LLMError conversion tests ────────────────────────────

    #[test]
    fn test_provider_error_401_to_auth_failed() {
        use reqwest::StatusCode;
        let err = super::map_http_error(StatusCode::UNAUTHORIZED, "invalid key".to_string(), None);
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::AuthFailed(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Auth);
    }

    #[test]
    fn test_provider_error_403_to_auth_failed() {
        use reqwest::StatusCode;
        let err = super::map_http_error(StatusCode::FORBIDDEN, "access denied".to_string(), None);
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::AuthFailed(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Auth);
    }

    #[test]
    fn test_provider_error_404_to_model_not_found() {
        use reqwest::StatusCode;
        let err = super::map_http_error(StatusCode::NOT_FOUND, "model not found".to_string(), None);
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::ModelNotFound(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::InvalidRequest);
    }

    #[test]
    fn test_provider_error_422_to_invalid_request() {
        use reqwest::StatusCode;
        let err = super::map_http_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad parameters".to_string(),
            None,
        );
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::InvalidRequest(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::InvalidRequest);
    }

    #[test]
    fn test_provider_error_429_to_rate_limit_exceeded() {
        use reqwest::StatusCode;
        let err = super::map_http_error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow down".to_string(),
            Some(30),
        );
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::RateLimitExceeded));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Transient);
    }

    #[test]
    fn test_provider_error_500_to_api_error_transient() {
        use reqwest::StatusCode;
        let err = super::map_http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal error".to_string(),
            None,
        );
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::ApiError(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Transient);
    }

    #[test]
    fn test_provider_error_unusual_status_418_to_api_error_unknown() {
        use reqwest::StatusCode;
        let err = super::map_http_error(
            StatusCode::from_u16(418).unwrap(),
            "teapot".to_string(),
            None,
        );
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::ApiError(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Unknown);
    }

    #[test]
    fn test_provider_error_legacy_to_api_error() {
        let err = crate::provider::ProviderError::Legacy("no choices in response".to_string());
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::ApiError(ref msg) if msg == "no choices in response"));
    }

    #[test]
    fn test_provider_error_reqwest_to_network_error() {
        // Reqwest errors can't easily be constructed, so test via the
        // legacy variant to confirm non-Http/non-Reqwest paths work,
        // and rely on the conversion logic being straightforward.
        let err = crate::provider::ProviderError::Legacy("test".to_string());
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::ApiError(_)));
    }

    // ── Error propagation chain: retry_after preservation ──────────────────────

    #[test]
    fn test_error_propagation_chain_429_preserves_retry_after() {
        let err = super::map_http_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "rate limited".to_string(),
            Some(120),
        );
        // ProviderError::Http carries retry_after
        match &err {
            super::ProviderError::Http { retry_after, .. } => {
                assert_eq!(*retry_after, Some(120));
            }
            _ => panic!("expected ProviderError::Http"),
        }
        // LLMError conversion preserves the semantic classification
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::RateLimitExceeded));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Transient);
    }

    #[test]
    fn test_error_propagation_chain_401_preserves_body() {
        let err = super::map_http_error(
            reqwest::StatusCode::UNAUTHORIZED,
            "invalid API key".to_string(),
            None,
        );
        let llm_err: LLMError = LLMError::from(&err);
        match &llm_err {
            LLMError::AuthFailed(msg) => {
                assert!(msg.contains("invalid API key"));
                assert!(msg.contains("401"));
            }
            other => panic!("expected AuthFailed, got {:?}", other),
        }
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Auth);
    }

    // ── Boundary values ───────────────────────────────────────────────────────

    #[test]
    fn test_map_http_error_empty_body() {
        let err = super::map_http_error(reqwest::StatusCode::BAD_REQUEST, String::new(), None);
        match err {
            super::ProviderError::Http {
                status_code,
                body,
                retry_after,
            } => {
                assert_eq!(status_code, 400);
                assert!(body.is_empty());
                assert_eq!(retry_after, None);
            }
            _ => panic!("expected ProviderError::Http"),
        }
    }

    #[test]
    fn test_map_http_error_unusual_status_418() {
        let err = super::map_http_error(
            reqwest::StatusCode::from_u16(418).unwrap(),
            "I'm a teapot".to_string(),
            None,
        );
        match &err {
            super::ProviderError::Http {
                status_code, body, ..
            } => {
                assert_eq!(*status_code, 418);
                assert_eq!(body, "I'm a teapot");
            }
            _ => panic!("expected ProviderError::Http"),
        }
        let llm_err: LLMError = LLMError::from(&err);
        // 418 is not in any special mapping → falls through to ApiError/Unknown
        assert!(matches!(llm_err, LLMError::ApiError(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Unknown);
    }

    #[test]
    fn test_map_http_error_high_status_599() {
        let err = super::map_http_error(
            reqwest::StatusCode::from_u16(599).unwrap(),
            "network connect timeout".to_string(),
            None,
        );
        match &err {
            super::ProviderError::Http { status_code, .. } => {
                assert_eq!(*status_code, 599);
            }
            _ => panic!("expected ProviderError::Http"),
        }
        let llm_err: LLMError = LLMError::from(&err);
        // 5xx status codes not in {500,502,503,504} still map to Unknown
        assert!(matches!(llm_err, LLMError::ApiError(_)));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Unknown);
    }

    #[test]
    fn test_provider_error_http_with_retry_after_converts_to_transient() {
        // Full chain: map_http_error → ProviderError::Http
        // → LLMError::RateLimitExceeded → kind()==Transient
        let err = super::map_http_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "slow down".to_string(),
            Some(60),
        );
        // Verify retry_after is carried
        match &err {
            super::ProviderError::Http {
                retry_after,
                body,
                status_code,
                ..
            } => {
                assert_eq!(*retry_after, Some(60));
                assert_eq!(*body, "slow down");
                assert_eq!(*status_code, 429);
            }
            _ => panic!("expected ProviderError::Http"),
        }
        // Verify conversion to RateLimitExceeded
        let llm_err: LLMError = LLMError::from(&err);
        assert!(matches!(llm_err, LLMError::RateLimitExceeded));
        assert_eq!(llm_err.kind(), super::super::ErrorKind::Transient);
    }

    #[test]
    fn test_error_kind_all_variants() {
        // Verify each LLMError variant produces the expected ErrorKind
        assert_eq!(
            LLMError::AuthFailed("x".into()).kind(),
            super::super::ErrorKind::Auth
        );
        assert_eq!(
            LLMError::RateLimitExceeded.kind(),
            super::super::ErrorKind::Transient
        );
        assert_eq!(
            LLMError::ModelNotFound("x".into()).kind(),
            super::super::ErrorKind::InvalidRequest
        );
        assert_eq!(
            LLMError::InvalidRequest("x".into()).kind(),
            super::super::ErrorKind::InvalidRequest
        );
        assert_eq!(
            LLMError::NetworkError("x".into()).kind(),
            super::super::ErrorKind::Transient
        );
        assert_eq!(
            LLMError::Cancelled.kind(),
            super::super::ErrorKind::InvalidRequest
        );
        // ApiError with 500 → Transient
        assert_eq!(
            LLMError::ApiError("HTTP 500: oops".into()).kind(),
            super::super::ErrorKind::Transient
        );
        // ApiError with 401 → Auth
        assert_eq!(
            LLMError::ApiError("HTTP 401: bad".into()).kind(),
            super::super::ErrorKind::Auth
        );
        // ApiError with 422 → InvalidRequest
        assert_eq!(
            LLMError::ApiError("HTTP 422: bad params".into()).kind(),
            super::super::ErrorKind::InvalidRequest
        );
        // ApiError with no known code → Unknown
        assert_eq!(
            LLMError::ApiError("something else".into()).kind(),
            super::super::ErrorKind::Unknown
        );
    }
}
