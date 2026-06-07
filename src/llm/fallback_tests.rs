//! Tests for LLM fallback chain client.

use crate::llm::fallback::{FallbackClient, ModelEntry};
use crate::llm::provider::Provider;
use crate::llm::types::{InternalResponse, ProtocolId, RawContentBlock, RawUsage};
use crate::llm::{ChatRequest, LLMError};
use std::sync::Arc;
use std::time::Duration;

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
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    response: Result<crate::llm::ChatResponse, LLMError>,
) -> crate::llm::provider::Result<InternalResponse> {
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
        Err(e) => Err(crate::llm::provider::ProviderError::Legacy(format!("{e}"))),
    }
}

struct MockProvider {
    name: String,
    response_fn: Box<dyn Fn() -> Result<crate::llm::ChatResponse, LLMError> + Send + Sync>,
}

impl MockProvider {
    fn new(name: &str, response: Result<crate::llm::ChatResponse, LLMError>) -> Self {
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
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        CLIENT.get_or_init(reqwest::Client::new)
    }

    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        static HEADERS: std::sync::OnceLock<reqwest::header::HeaderMap> =
            std::sync::OnceLock::new();
        HEADERS.get_or_init(reqwest::header::HeaderMap::new)
    }

    async fn send(
        &self,
        _request: crate::llm::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::llm::provider::Result<InternalResponse> {
        chat_to_internal((self.response_fn)())
    }

    async fn send_streaming(
        &self,
        _request: crate::llm::types::InternalRequest,
        _body: serde_json::Value,
    ) -> crate::llm::provider::Result<crate::llm::provider::SseStream> {
        unimplemented!("streaming not needed in fallback tests")
    }
}

/// Wrap a MockProvider into an Arc<dyn Provider>.
fn mock_provider_as_dyn(
    name: &str,
    response: Result<crate::llm::ChatResponse, LLMError>,
) -> Arc<dyn Provider> {
    Arc::new(MockProvider::new(name, response))
}

fn ok_response() -> crate::llm::ChatResponse {
    crate::llm::ChatResponse {
        model: "test-model".to_string(),
        content: "hello".to_string(),
        usage: crate::llm::Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        },
    }
}

#[tokio::test]
async fn test_fallback_client_succeeds_on_first_model() {
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    assert_eq!(result.unwrap().content, "hello");
}

#[tokio::test]
async fn test_fallback_client_falls_through_on_auth_error() {
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    let registry = Arc::new(crate::llm::LLMRegistry::new());
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
    let registry = Arc::new(crate::llm::LLMRegistry::new());
    let client = FallbackClient::from_strings(
        registry,
        vec!["valid/model".to_string(), "no-slash".to_string()],
    );
    assert_eq!(client.fallback_chain.len(), 1);
}

#[test]
fn test_with_timeout() {
    let registry = Arc::new(crate::llm::LLMRegistry::new());
    let client = FallbackClient::new(registry, vec![]).with_timeout(60);
    assert_eq!(client.call_timeout, Duration::from_secs(60));
}
