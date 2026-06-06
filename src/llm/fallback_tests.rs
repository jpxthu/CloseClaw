//! Tests for LLM fallback chain client.

use crate::llm::fallback::{FallbackClient, ModelEntry};
use crate::llm::legacy::legacy_provider::LegacyProviderBridge;
use crate::llm::provider::Provider;
use crate::llm::{ChatRequest, ChatResponse, LLMError, LLMProvider};
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

struct MockProvider {
    name: String,
    response_fn: Box<dyn Fn() -> Result<ChatResponse, LLMError> + Send + Sync>,
}

impl MockProvider {
    fn new(name: &str, response: Result<ChatResponse, LLMError>) -> Self {
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
impl LLMProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn models(&self) -> Vec<&str> {
        vec!["test-model"]
    }
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LLMError> {
        (self.response_fn)()
    }
}

/// Wrap a MockProvider into an Arc<dyn Provider> via LegacyProviderBridge.
fn mock_provider_as_dyn(name: &str, response: Result<ChatResponse, LLMError>) -> Arc<dyn Provider> {
    let mock = MockProvider::new(name, response);
    Arc::new(LegacyProviderBridge::new(
        mock,
        String::new(),
        String::new(),
        vec![],
        reqwest::Client::new(),
        reqwest::header::HeaderMap::new(),
    ))
}

fn ok_response() -> ChatResponse {
    ChatResponse {
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
