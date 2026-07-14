//! Tests for LLM fallback chain client.

use crate::fallback::{FallbackClient, ModelEntry};
use crate::provider::Provider;
use crate::types::{InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage};
use crate::{ChatRequest, LLMError};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[test]
fn test_model_entry_parse() {
    let entry = ModelEntry {
        provider: "minimax".to_string(),
        model: "MiniMax-M2.7".to_string(),
    };
    assert_eq!(entry.provider, "minimax");
    assert_eq!(entry.model, "MiniMax-M2.7");
}

#[tokio::test]
async fn test_fallback_client_requires_registry() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let client = FallbackClient::from_strings(registry, vec![]);
    let req = ChatRequest {
        model: "MiniMax-M2.7".to_string(),
        messages: vec![],
        temperature: 0.7,
        max_tokens: Some(100),
    };
    let err = client.chat(req).await.unwrap_err();
    assert!(err.to_string().contains("exhausted"));
}

// --- Mock provider for fallback chain tests ---

/// Convert a chat-style response/error pair into the internal response type
/// that the Provider trait expects.
fn chat_to_internal(
    response: Result<crate::ChatResponse, LLMError>,
) -> crate::provider::Result<InternalResponse> {
    match response {
        Ok(resp) => Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(resp.content)],
            usage: RawUsage {
                prompt_tokens: resp.usage.prompt_tokens,
                completion_tokens: resp.usage.completion_tokens,
                total_tokens: Some(resp.usage.total_tokens),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        }),
        Err(e) => Err(crate::provider::ProviderError::Legacy(format!("{e}"))),
    }
}

struct MockProvider {
    name: String,
    response_fn: Box<dyn Fn() -> Result<crate::ChatResponse, LLMError> + Send + Sync>,
}

impl MockProvider {
    fn new(name: &str, response: Result<crate::ChatResponse, LLMError>) -> Self {
        let r = Arc::new(response);
        Self {
            name: name.to_string(),
            response_fn: Box::new(move || match Arc::as_ref(&r) {
                Ok(v) => Ok(v.clone()),
                Err(e) => {
                    // Reconstruct error since LLMError isn't Clone
                    match e {
                        LLMError::AuthFailed(msg) => Err(LLMError::AuthFailed(msg.clone())),
                        LLMError::RateLimitExceeded => Err(LLMError::RateLimitExceeded),
                        LLMError::ModelNotFound(msg) => Err(LLMError::ModelNotFound(msg.clone())),
                        LLMError::InvalidRequest(msg) => Err(LLMError::InvalidRequest(msg.clone())),
                        LLMError::ApiError(msg) => Err(LLMError::ApiError(msg.clone())),
                        LLMError::NetworkError(msg) => Err(LLMError::NetworkError(msg.clone())),
                        LLMError::Cancelled => Err(LLMError::Cancelled),
                    }
                }
            }),
        }
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str {
        &self.name
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn api_key(&self) -> &str {
        ""
    }

    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }

    fn http_client(&self) -> &reqwest::Client {
        mock_provider_client()
    }

    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        mock_provider_headers()
    }

    async fn send(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<InternalResponse> {
        chat_to_internal((self.response_fn)())
    }

    async fn send_streaming(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        unimplemented!("streaming not needed in fallback tests")
    }
}

/// Wrap a MockProvider into an Arc<dyn Provider>.
fn mock_provider_as_dyn(
    name: &str,
    response: Result<crate::ChatResponse, LLMError>,
) -> Arc<dyn Provider> {
    Arc::new(MockProvider::new(name, response))
}

fn ok_response() -> crate::ChatResponse {
    crate::ChatResponse {
        model: "test-model".to_string(),
        content: "hello".to_string(),
        usage: crate::Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        },
    }
}

#[tokio::test]
async fn test_fallback_client_succeeds_on_first_model() {
    let registry = Arc::new(crate::LLMRegistry::new());
    registry
        .register(
            "prov".to_string(),
            mock_provider_as_dyn("prov", Ok(ok_response())),
        )
        .await;

    let client = FallbackClient::from_strings(registry, vec!["prov/test-model".to_string()]);
    let req = ChatRequest {
        model: "test-model".to_string(),
        messages: vec![],
        temperature: 0.7,
        max_tokens: Some(100),
    };
    let result = client.chat(req).await;
    assert!(result.is_ok());
    let (resp, _retries) = result.unwrap();
    assert_eq!(resp.content, "hello");
}

#[tokio::test]
async fn test_fallback_client_falls_through_on_auth_error() {
    let registry = Arc::new(crate::LLMRegistry::new());
    // First provider fails with auth error
    registry
        .register(
            "fail".to_string(),
            mock_provider_as_dyn("fail", Err(LLMError::AuthFailed("bad key".to_string()))),
        )
        .await;
    // Second provider succeeds
    registry
        .register(
            "ok".to_string(),
            mock_provider_as_dyn("ok", Ok(ok_response())),
        )
        .await;

    let client = FallbackClient::new(
        registry,
        vec![
            ModelEntry {
                provider: "fail".to_string(),
                model: "m1".to_string(),
            },
            ModelEntry {
                provider: "ok".to_string(),
                model: "m2".to_string(),
            },
        ],
    );
    let req = ChatRequest {
        model: "m1".to_string(),
        messages: vec![],
        temperature: 0.7,
        max_tokens: Some(100),
    };
    let result = client.chat(req).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_fallback_client_skips_missing_provider() {
    let registry = Arc::new(crate::LLMRegistry::new());
    registry
        .register(
            "ok".to_string(),
            mock_provider_as_dyn("ok", Ok(ok_response())),
        )
        .await;

    let client = FallbackClient::new(
        registry,
        vec![
            ModelEntry {
                provider: "missing".to_string(),
                model: "m1".to_string(),
            },
            ModelEntry {
                provider: "ok".to_string(),
                model: "m2".to_string(),
            },
        ],
    );
    let req = ChatRequest {
        model: "m1".to_string(),
        messages: vec![],
        temperature: 0.7,
        max_tokens: Some(100),
    };
    let result = client.chat(req).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_fallback_client_all_exhausted() {
    let registry = Arc::new(crate::LLMRegistry::new());
    registry
        .register(
            "fail".to_string(),
            mock_provider_as_dyn("fail", Err(LLMError::InvalidRequest("bad".to_string()))),
        )
        .await;

    let client = FallbackClient::from_strings(registry, vec!["fail/test-model".to_string()]);
    let req = ChatRequest {
        model: "test-model".to_string(),
        messages: vec![],
        temperature: 0.7,
        max_tokens: Some(100),
    };
    let err = client.chat(req).await.unwrap_err();
    assert!(err.to_string().contains("exhausted"));
}

#[test]
fn test_from_strings_parses_provider_model() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let client = FallbackClient::from_strings(
        registry,
        vec!["prov-a/model-1".to_string(), "prov-b/model-2".to_string()],
    );
    assert_eq!(client.fallback_chain.len(), 2);
    assert_eq!(client.fallback_chain[0].provider, "prov-a");
    assert_eq!(client.fallback_chain[1].model, "model-2");
}

#[test]
fn test_from_strings_skips_invalid() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let client = FallbackClient::from_strings(
        registry,
        vec!["valid/model".to_string(), "no-slash".to_string()],
    );
    assert_eq!(client.fallback_chain.len(), 1);
}

#[test]
fn test_with_timeout() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let client = FallbackClient::new(registry, vec![]).with_timeout(60);
    assert_eq!(client.call_timeout, Duration::from_secs(60));
}

// ============================================================================
// Streaming tests for FallbackClient::chat_streaming
// ============================================================================

use crate::types::{InternalMessage, InternalRequest};
use closeclaw_session::persistence::ReasoningLevel;
use futures::StreamExt;

/// Build a minimal InternalRequest for streaming tests.
fn streaming_request(model: &str) -> InternalRequest {
    InternalRequest {
        model: model.to_string(),
        messages: vec![InternalMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
            ..Default::default()
        }],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

/// Create an SSE chunk with the given JSON data.
fn sse_chunk(data: &str) -> RawSseChunk {
    RawSseChunk {
        event_type: "message".to_string(),
        data: data.to_string(),
    }
}

/// Build OpenAI-style streaming JSON for a single content delta.
fn openai_content_delta(text: &str) -> String {
    serde_json::json!({
        "choices": [{ "delta": { "content": text } }]
    })
    .to_string()
}

// --- Mock providers for streaming tests ---

fn mock_provider_headers() -> &'static reqwest::header::HeaderMap {
    static HEADERS: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
    HEADERS.get_or_init(reqwest::header::HeaderMap::new)
}

fn mock_provider_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// A provider that streams SSE chunks and also supports non-streaming.
struct StreamingProvider {
    name: String,
    /// Text chunks to emit via streaming.
    chunks: Vec<String>,
    /// Non-streaming response text (used by degraded path).
    fallback_text: String,
    /// Error to return from send_streaming (if set).
    streaming_error: Option<String>,
}

impl StreamingProvider {
    fn streaming_only(name: &str, chunks: Vec<String>) -> Self {
        let fallback_text = chunks.join("");
        Self {
            name: name.to_string(),
            chunks,
            fallback_text,
            streaming_error: None,
        }
    }

    fn streaming_fail(send_ok: bool, fallback_text: &str) -> Self {
        Self {
            name: "fail-stream".to_string(),
            chunks: vec![],
            fallback_text: fallback_text.to_string(),
            streaming_error: if send_ok {
                None
            } else {
                Some("streaming not supported".to_string())
            },
        }
    }
}

#[async_trait::async_trait]
impl Provider for StreamingProvider {
    fn id(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        ""
    }
    fn api_key(&self) -> &str {
        ""
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }
    fn http_client(&self) -> &reqwest::Client {
        mock_provider_client()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        mock_provider_headers()
    }

    async fn send(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(self.fallback_text.clone())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: Some(0),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }

    async fn send_streaming(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        if let Some(ref msg) = self.streaming_error {
            return Err(crate::provider::ProviderError::Legacy(msg.clone()));
        }
        let (tx, rx) = mpsc::channel(32);
        let chunks = self.chunks.clone();
        tokio::spawn(async move {
            for text in &chunks {
                let data = openai_content_delta(text);
                let _ = tx.send(sse_chunk(&data)).await;
            }
            let _ = tx.send(sse_chunk("[DONE]")).await;
        });
        Ok(rx)
    }
}

/// A provider whose send_streaming blocks forever (simulates timeout).
struct HangingStreamingProvider {
    name: String,
    fallback_text: String,
}

#[async_trait::async_trait]
impl Provider for HangingStreamingProvider {
    fn id(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        ""
    }
    fn api_key(&self) -> &str {
        ""
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }
    fn http_client(&self) -> &reqwest::Client {
        mock_provider_client()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        mock_provider_headers()
    }

    async fn send(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(self.fallback_text.clone())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: Some(0),
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }

    async fn send_streaming(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        // Block forever — the caller's tokio::time::timeout should cancel this.
        std::future::pending().await
    }
}

/// A provider that always fails both send and send_streaming.
struct AlwaysFailProvider {
    name: String,
}

#[async_trait::async_trait]
impl Provider for AlwaysFailProvider {
    fn id(&self) -> &str {
        &self.name
    }
    fn base_url(&self) -> &str {
        ""
    }
    fn api_key(&self) -> &str {
        ""
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }
    fn http_client(&self) -> &reqwest::Client {
        mock_provider_client()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        mock_provider_headers()
    }

    async fn send(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<InternalResponse> {
        Err(crate::provider::ProviderError::Legacy(
            "always fails".to_string(),
        ))
    }

    async fn send_streaming(
        &self,
        _request: crate::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        Err(crate::provider::ProviderError::Legacy(
            "always fails".to_string(),
        ))
    }
}

/// Register a provider and return the model entry.
async fn register_provider(
    registry: &crate::LLMRegistry,
    provider: Arc<dyn Provider>,
    model: &str,
) -> ModelEntry {
    let provider_name = provider.id().to_string();
    registry.register(provider_name.clone(), provider).await;
    ModelEntry {
        provider: provider_name,
        model: model.to_string(),
    }
}

// --- Test: normal streaming path ---

#[tokio::test]
async fn test_streaming_normal_path() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let provider = Arc::new(StreamingProvider::streaming_only(
        "stream-ok",
        vec!["hel".into(), "lo".into()],
    ));
    let entry = register_provider(&registry, provider, "m1").await;

    let client = FallbackClient::new(registry, vec![entry]);
    let request = streaming_request("m1");
    let result = client.chat_streaming(request).await;
    assert!(result.is_ok(), "streaming call should succeed");

    let mut stream = result.unwrap();
    let mut texts = Vec::new();
    while let Some(event) = stream.next().await {
        let ev = event.unwrap();
        match ev {
            crate::types::StreamEvent::BlockDelta { delta, .. } => {
                if let crate::types::ContentDelta::Text { text } = delta {
                    texts.push(text);
                }
            }
            crate::types::StreamEvent::BlockStart { .. }
            | crate::types::StreamEvent::BlockEnd { .. } => {}
            crate::types::StreamEvent::MessageEnd { .. } => break,
            _ => {}
        }
    }
    assert_eq!(texts, vec!["hel", "lo"], "should emit content deltas");
}

// --- Test: streaming fails, degrades to non-streaming ---

#[tokio::test]
async fn test_streaming_degraded_to_non_streaming() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let provider = Arc::new(StreamingProvider::streaming_fail(
        false,
        "degraded response",
    ));
    let entry = register_provider(&registry, provider, "m1").await;

    let client = FallbackClient::new(registry, vec![entry]);
    let request = streaming_request("m1");
    let result = client.chat_streaming(request).await;
    assert!(result.is_ok(), "should degrade to non-streaming");

    let mut stream = result.unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
    }
    assert!(!events.is_empty(), "degraded stream should produce events");
    // Last event should be MessageEnd
    let last = events.last().unwrap();
    assert!(
        matches!(last, crate::types::StreamEvent::MessageEnd { .. }),
        "last event should be MessageEnd"
    );
}

// --- Test: all providers fail streaming, and degraded non-streaming also fails ---

#[tokio::test]
async fn test_streaming_all_fail_returns_error() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let provider = Arc::new(AlwaysFailProvider {
        name: "always-fail".to_string(),
    });
    let entry = register_provider(&registry, provider, "m1").await;

    let client = FallbackClient::new(registry, vec![entry]);
    let request = streaming_request("m1");
    let result = client.chat_streaming(request).await;
    assert!(
        result.is_err(),
        "should return error when all providers fail"
    );
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(
        msg.contains("exhausted"),
        "error should indicate chain exhausted, got: {}",
        msg
    );
}

// --- Test: timeout on streaming provider, falls through to next ---

#[tokio::test]
async fn test_streaming_timeout_falls_through() {
    let registry = Arc::new(crate::LLMRegistry::new());

    let hanging = Arc::new(HangingStreamingProvider {
        name: "hanging".to_string(),
        fallback_text: "should not be used".to_string(),
    });
    let entry_hanging = register_provider(&registry, hanging, "m-hang").await;

    let ok = Arc::new(StreamingProvider::streaming_only(
        "ok",
        vec!["from-ok".into()],
    ));
    let entry_ok = register_provider(&registry, ok, "m-ok").await;

    let client = FallbackClient::new(registry, vec![entry_hanging, entry_ok]).with_timeout(1); // 1 second timeout

    let request = streaming_request("m-hang");
    let result = client.chat_streaming(request).await;
    assert!(result.is_ok(), "should succeed via second provider");

    let mut stream = result.unwrap();
    let mut texts = Vec::new();
    while let Some(event) = stream.next().await {
        let ev = event.unwrap();
        match ev {
            crate::types::StreamEvent::BlockDelta { delta, .. } => {
                if let crate::types::ContentDelta::Text { text } = delta {
                    texts.push(text);
                }
            }
            crate::types::StreamEvent::MessageEnd { .. } => break,
            _ => {}
        }
    }
    assert_eq!(
        texts,
        vec!["from-ok"],
        "should get text from second provider"
    );
}

// --- Test: streaming chain traversal, first fails, second succeeds ---

#[tokio::test]
async fn test_streaming_chain_traversal() {
    let registry = Arc::new(crate::LLMRegistry::new());

    let fail = Arc::new(StreamingProvider::streaming_fail(false, "fallback-text"));
    let entry_fail = register_provider(&registry, fail, "m-fail").await;

    let ok = Arc::new(StreamingProvider::streaming_only(
        "ok",
        vec!["chunk-a".into(), "chunk-b".into()],
    ));
    let entry_ok = register_provider(&registry, ok, "m-ok").await;

    let client = FallbackClient::new(registry, vec![entry_fail, entry_ok]);
    let request = streaming_request("m-fail");
    let result = client.chat_streaming(request).await;
    assert!(result.is_ok(), "should succeed via second provider");

    let mut stream = result.unwrap();
    let mut texts = Vec::new();
    while let Some(event) = stream.next().await {
        let ev = event.unwrap();
        match ev {
            crate::types::StreamEvent::BlockDelta { delta, .. } => {
                if let crate::types::ContentDelta::Text { text } = delta {
                    texts.push(text);
                }
            }
            crate::types::StreamEvent::MessageEnd { .. } => break,
            _ => {}
        }
    }
    assert_eq!(
        texts,
        vec!["chunk-a", "chunk-b"],
        "should get chunks from second provider"
    );
}

// --- Test: empty chain returns degraded non-streaming error ---

#[tokio::test]
async fn test_streaming_empty_chain_error() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let client = FallbackClient::new(registry, vec![]);
    let request = streaming_request("m1");
    let result = client.chat_streaming(request).await;
    assert!(result.is_err(), "empty chain should fail");
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(msg.contains("exhausted"), "unexpected error: {}", msg);
}

// --- Test: all entries fail streaming but non-streaming also fails ---

#[tokio::test]
async fn test_streaming_degraded_non_streaming_also_fails() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let provider = Arc::new(AlwaysFailProvider {
        name: "both-fail".to_string(),
    });
    let entry = register_provider(&registry, provider, "m1").await;

    let client = FallbackClient::new(registry, vec![entry]);
    let request = streaming_request("m1");
    let result = client.chat_streaming(request).await;
    assert!(result.is_err());
    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(msg.contains("exhausted"));
}
