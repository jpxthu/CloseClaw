#![allow(deprecated)]

//! Tests for LLM fallback chain client.

use crate::fallback::{FallbackClient, ModelEntry};
use crate::provider::Provider;
use crate::test_support::isolated_cooldown;
use crate::types::ProtocolId;
use crate::{ChatRequest, LLMError};
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
    let registry = Arc::new(crate::LLMRegistry::new());
    let (_dir, client) = isolated_client_from_strings(registry, vec![]);
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

/// Convert a chat-style response/error pair into raw JSON
/// that the Provider trait expects (Protocol layer parses it).
fn chat_to_raw_json(
    response: Result<crate::ChatResponse, LLMError>,
) -> crate::provider::Result<serde_json::Value> {
    match response {
        Ok(resp) => Ok(serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": resp.content },
                "finish_reason": null
            }],
            "usage": {
                "prompt_tokens": resp.usage.prompt_tokens,
                "completion_tokens": resp.usage.completion_tokens,
                "total_tokens": resp.usage.total_tokens
            }
        })),
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
                        LLMError::PartialContent {
                            reason,
                            thinking_blocks,
                        } => Err(LLMError::PartialContent {
                            reason: reason.clone(),
                            thinking_blocks: thinking_blocks.clone(),
                        }),
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
    ) -> crate::provider::Result<serde_json::Value> {
        chat_to_raw_json((self.response_fn)())
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

/// Static empty header map shared by mock providers.
pub(crate) fn mock_provider_headers() -> &'static reqwest::header::HeaderMap {
    static HEADERS: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
    HEADERS.get_or_init(reqwest::header::HeaderMap::new)
}

/// Static HTTP client shared by mock providers.
pub(crate) fn mock_provider_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// FallbackClient with a TempDir-backed cooldown manager (never touches real ~/.closeclaw).
///
/// The returned `TempDir` must be kept alive for as long as the client is used.
pub(crate) fn isolated_client(
    registry: Arc<crate::LLMRegistry>,
    chain: Vec<ModelEntry>,
) -> (tempfile::TempDir, FallbackClient) {
    let (dir, cooldown) = isolated_cooldown();
    (
        dir,
        FallbackClient::new_with_cooldown(registry, chain, cooldown),
    )
}

/// Same as [`isolated_client`] but builds the chain from config-style strings.
fn isolated_client_from_strings(
    registry: Arc<crate::LLMRegistry>,
    chain: Vec<String>,
) -> (tempfile::TempDir, FallbackClient) {
    let (dir, cooldown) = isolated_cooldown();
    (
        dir,
        FallbackClient::from_strings_with_cooldown(registry, chain, cooldown),
    )
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

    let (_dir, client) =
        isolated_client_from_strings(registry, vec!["prov/test-model".to_string()]);
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

    let (_dir, client) = isolated_client(
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

    let (_dir, client) = isolated_client(
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

    let (_dir, client) =
        isolated_client_from_strings(registry, vec!["fail/test-model".to_string()]);
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
    let (_dir, client) = isolated_client_from_strings(
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
    let (_dir, client) = isolated_client_from_strings(
        registry,
        vec!["valid/model".to_string(), "no-slash".to_string()],
    );
    assert_eq!(client.fallback_chain.len(), 1);
}

#[test]
fn test_with_timeout() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let (_dir, client) = isolated_client(registry, vec![]);
    let client = client.with_timeout(60);
    assert_eq!(client.call_timeout, Duration::from_secs(60));
}

// Structured error propagation tests (Step 1.4)
struct HttpErrorProvider {
    name: String,
    status: u16,
    body: String,
    retry_after: Option<u64>,
}

impl HttpErrorProvider {
    fn new(name: &str, status: u16, body: &str) -> Self {
        Self {
            name: name.to_string(),
            status,
            body: body.to_string(),
            retry_after: None,
        }
    }
}

#[async_trait::async_trait]
impl Provider for HttpErrorProvider {
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
        _: crate::types::InternalRequest,
        _: serde_json::Value,
    ) -> crate::provider::Result<serde_json::Value> {
        Err(crate::provider::ProviderError::Http {
            status_code: self.status,
            body: self.body.clone(),
            retry_after: self.retry_after,
        })
    }
    async fn send_streaming(
        &self,
        _: crate::types::InternalRequest,
        _: serde_json::Value,
    ) -> crate::provider::Result<crate::provider::SseStream> {
        Err(crate::provider::ProviderError::Http {
            status_code: self.status,
            body: self.body.clone(),
            retry_after: self.retry_after,
        })
    }
}

fn structured_internal_request(model: &str) -> crate::types::InternalRequest {
    crate::types::InternalRequest {
        model: model.to_string(),
        messages: vec![],
        temperature: 0.0,
        max_tokens: None,
        stream: false,
        extra_body: serde_json::Map::new(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: closeclaw_session::persistence::ReasoningLevel::default(),
        turn_count: None,
    }
}

fn http_error_client(name: &str, status: u16, body: &str) -> crate::client::UnifiedChatClient {
    use crate::cache_adapter::NoopCacheAdapter;
    use crate::client::UnifiedChatClient;
    use crate::interpreter::InterpreterRegistry;
    use crate::plugin::PluginPipeline;
    use crate::protocol::OpenAiProtocol;
    use std::sync::Arc;
    let provider = Arc::new(HttpErrorProvider::new(name, status, body));
    UnifiedChatClient::new(
        provider,
        Arc::new(OpenAiProtocol::default()),
        InterpreterRegistry::new(vec![]),
        PluginPipeline::new(),
        Arc::new(NoopCacheAdapter),
    )
}

/// Parameterized: all status code → LLMError variant/kind mappings.
#[tokio::test]
async fn test_structured_error_status_code_mapping() {
    // (status, body, expected_variant, expected_kind)
    let cases: [(u16, &str, &str, crate::ErrorKind); 7] = [
        (
            429,
            "rate limited",
            "RateLimitExceeded",
            crate::ErrorKind::Transient,
        ),
        (401, "invalid key", "AuthFailed", crate::ErrorKind::Auth),
        (403, "forbidden", "AuthFailed", crate::ErrorKind::Auth),
        (
            404,
            "not found",
            "ModelNotFound",
            crate::ErrorKind::InvalidRequest,
        ),
        (
            422,
            "bad params",
            "InvalidRequest",
            crate::ErrorKind::InvalidRequest,
        ),
        (500, "server error", "ApiError", crate::ErrorKind::Transient),
        (418, "teapot", "ApiError", crate::ErrorKind::Unknown),
    ];
    for (status, body, expected_variant, expected_kind) in cases {
        let client = http_error_client("prov", status, body);
        let req = structured_internal_request("m1");
        let err = client.chat(req).await.unwrap_err();
        match &err {
            crate::client::ClientError::Provider(crate::provider::ProviderError::Http {
                status_code,
                ..
            }) => assert_eq!(*status_code, status),
            other => panic!("[{status}] expected ProviderError::Http, got {other:?}"),
        }
        let llm_err: LLMError = LLMError::from(err);
        assert_eq!(llm_err.kind(), expected_kind, "[{status}] kind mismatch");
        let name = match &llm_err {
            LLMError::AuthFailed(_) => "AuthFailed",
            LLMError::RateLimitExceeded => "RateLimitExceeded",
            LLMError::ModelNotFound(_) => "ModelNotFound",
            LLMError::InvalidRequest(_) => "InvalidRequest",
            LLMError::ApiError(_) => "ApiError",
            _ => "other",
        };
        assert_eq!(name, expected_variant, "[{status}] variant mismatch");
    }
}

#[tokio::test]
async fn test_structured_error_empty_body() {
    let client = http_error_client("prov", 429, "");
    let req = structured_internal_request("m1");
    let err = client.chat(req).await.unwrap_err();
    let llm_err: LLMError = LLMError::from(err);
    assert!(matches!(llm_err, LLMError::RateLimitExceeded));
    assert_eq!(llm_err.kind(), crate::ErrorKind::Transient);
}

#[tokio::test]
async fn test_structured_429_triggers_fallback_to_second_provider() {
    let registry = Arc::new(crate::LLMRegistry::new());
    let fail = Arc::new(HttpErrorProvider::new("rate-limited", 429, "slow down"));
    registry
        .register(fail.id().to_string(), fail as Arc<dyn Provider>)
        .await;
    registry
        .register(
            "ok-prov".to_string(),
            mock_provider_as_dyn("ok-prov", Ok(ok_response())),
        )
        .await;
    let (_dir, client) = isolated_client(
        registry,
        vec![
            ModelEntry {
                provider: "rate-limited".to_string(),
                model: "m1".to_string(),
            },
            ModelEntry {
                provider: "ok-prov".to_string(),
                model: "m2".to_string(),
            },
        ],
    );
    let req = ChatRequest {
        model: "m1".to_string(),
        messages: vec![],
        temperature: 0.0,
        max_tokens: None,
    };
    let result = client.chat_unified(req).await;
    assert!(result.is_ok(), "should succeed via second provider");
    let resp = result.unwrap();
    let content: String = resp
        .content_blocks
        .iter()
        .filter_map(|b| match b {
            closeclaw_common::processor::ContentBlock::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(content, "hello");
}
