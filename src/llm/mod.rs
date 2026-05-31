//! LLM Interface - Abstract trait for multiple LLM providers

pub mod adapter;
pub mod anthropic;
pub mod cache_adapter;
pub mod fallback;
pub mod glm;
pub mod glm_stream;
pub mod http_client;
pub mod knowledge;
pub mod minimax;
pub mod model_cache;
pub mod model_discovery;
pub mod model_info;
pub mod openai;
pub mod protocol;
pub mod provider;
pub mod retry;
pub mod session;
pub mod stats;
pub mod stub;
pub mod turn;
pub mod types;
#[cfg(test)]
mod types_tests;

pub mod deepseek;
pub mod volcengine;

pub mod client;
#[cfg(test)]
mod client_test;
pub mod interpreter;
#[cfg(test)]
mod interpreter_test;
pub mod plugin;

#[cfg(feature = "fake-llm")]
pub mod fake;
#[cfg(feature = "fake-llm")]
pub use fake::FakeProvider;

pub use anthropic::AnthropicProvider;
pub use deepseek::DeepSeekProvider;
pub use glm::GlmProvider;
pub use http_client::{HttpClient, ReqwestHttpClient};
pub use knowledge::{ModelRecommendParams, ProviderModelKnowledge, ReasoningLevels};
pub use minimax::MiniMaxProvider;
pub use model_cache::{CacheEntry, ModelCache};
pub use model_discovery::ModelDiscovery;
pub use model_info::{InputType, ModelInfo};
pub use openai::OpenAIProvider;
pub use protocol::ChatProtocol;
pub use provider::Provider;
pub use volcengine::VolcEngineProvider;

pub use stub::StubProvider;

pub use client::UnifiedChatClient;
pub use interpreter::{DefaultInterpreter, InterpreterRegistry, ModelInterpreter};
pub use plugin::PluginPipeline;
pub use session::{ChatSession, ConversationSession, SessionMessage};
pub use turn::TurnCounter;
pub use types::{InternalRequest, ProtocolId};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// LLM message
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// LLM chat completion request
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub temperature: f32,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// LLM chat completion response
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Usage,
}

/// Token usage
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// A single chunk from streaming chat response.
/// Each chunk contains a text fragment, or an error, or signals the end.
#[derive(Debug, Clone)]
pub enum ChatStreamChunk {
    /// A text fragment from the stream (delta content)
    Text(String),
    /// The stream ended with this final response metadata
    Done { model: String, usage: Usage },
    /// An error occurred during streaming
    Error(LLMError),
}

/// Streamed chat response receiver.
/// Callers consume chunks with `receiver.recv().await` until `None`.
pub type StreamingResponse = tokio::sync::mpsc::Receiver<ChatStreamChunk>;

/// LLM provider trait - implemented by each LLM provider
#[deprecated(
    note = "LLMProvider is superseded by the `Provider` trait in `crate::llm::Provider`. \
          Use LegacyProviderAdapter to bridge old providers to the new trait."
)]
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &str;

    /// Send a chat request
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError>;

    /// Send a streaming chat request. Default implementation wraps chat() as a single chunk.
    async fn chat_streaming(&self, request: ChatRequest) -> Result<StreamingResponse, LLMError> {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        let response = self.chat(request).await?;
        let _ = tx.send(ChatStreamChunk::Text(response.content)).await;
        let _ = tx
            .send(ChatStreamChunk::Done {
                model: response.model,
                usage: response.usage,
            })
            .await;
        Ok(rx)
    }

    /// List available models
    fn models(&self) -> Vec<&str>;

    /// Send a chat request and return a unified response with structured content blocks.
    /// Default implementation wraps `chat()` response as a single Text block.
    async fn chat_unified(
        &self,
        request: ChatRequest,
    ) -> Result<crate::llm::types::UnifiedResponse, LLMError> {
        let response = self.chat(request).await?;
        Ok(crate::llm::types::UnifiedResponse {
            content_blocks: vec![crate::llm::types::ContentBlock::Text(response.content)],
            usage: crate::llm::types::UnifiedUsage {
                prompt_tokens: response.usage.prompt_tokens,
                completion_tokens: response.usage.completion_tokens,
                total_tokens: Some(response.usage.total_tokens),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }

    /// Returns true if this is a stub provider that returns fake responses.
    /// When true, callers should treat this as a configuration error.
    fn is_stub(&self) -> bool {
        false
    }

    /// Human-readable display name for this provider.
    /// Defaults to `self.name()`.
    fn provider_display_name(&self) -> &str {
        self.name()
    }

    /// Fetch the list of available models from this provider via the API.
    /// Returns `ModelNotFound` by default, indicating the provider does not support
    /// dynamic model discovery.
    async fn fetch_model_list(&self, _bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError> {
        Err(LLMError::ModelNotFound(
            "fetch_model_list not supported by this provider".to_string(),
        ))
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum LLMError {
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Network error: {0}")]
    NetworkError(String),
}

/// Classifies an LLM error to determine retry strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Transient errors (429, 5xx, timeout) — retry with backoff
    Transient,
    /// Auth errors (401, 403) — rotate credentials, do not retry same credentials
    Auth,
    /// Billing errors (402, quota exhausted) — long cooldown
    Billing,
    /// Invalid request (400, 422) — do not retry, switch model
    InvalidRequest,
    /// Unknown errors — treat as transient with limited retries
    Unknown,
}

impl LLMError {
    /// Classify this error to determine retry strategy
    pub fn kind(&self) -> ErrorKind {
        use ErrorKind::*;
        match self {
            // Auth: credentials issue, no point retrying same credentials
            LLMError::AuthFailed(_) => Auth,
            // Rate limit — could be transient or billing
            LLMError::RateLimitExceeded => Transient,
            // Invalid request — don't retry, fix the request
            LLMError::InvalidRequest(_) | LLMError::ModelNotFound(_) => InvalidRequest,
            // API errors — check status if available; default to Transient
            LLMError::ApiError(msg) => {
                // Heuristic: messages containing status codes
                if msg.contains("500")
                    || msg.contains("502")
                    || msg.contains("503")
                    || msg.contains("504")
                {
                    Transient
                } else if msg.contains("400") || msg.contains("422") {
                    InvalidRequest
                } else if msg.contains("401") || msg.contains("403") {
                    Auth
                } else {
                    Unknown
                }
            }
            // Network errors are transient
            LLMError::NetworkError(_) => Transient,
        }
    }
}

/// LLM Registry - manages multiple providers
pub struct LLMRegistry {
    providers: tokio::sync::RwLock<std::collections::HashMap<String, Arc<dyn LLMProvider>>>,
}

impl Default for LLMRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LLMRegistry {
    pub fn new() -> Self {
        Self {
            providers: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }

    pub async fn register(&self, name: String, provider: Arc<dyn LLMProvider>) {
        let mut providers = self.providers.write().await;
        providers.insert(name, provider);
    }

    pub async fn get(&self, name: &str) -> Option<Arc<dyn LLMProvider>> {
        let providers = self.providers.read().await;
        providers.get(name).cloned()
    }

    pub async fn list(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        providers.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::stub::StubProvider;

    #[test]
    fn test_message_serde() {
        let msg = Message {
            role: "user".to_string(),
            content: "Hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.content, "Hello");
    }

    #[tokio::test]
    async fn test_registry() {
        let registry = LLMRegistry::new();
        // Registry should start empty
        let providers = registry.list().await;
        assert!(providers.is_empty());
    }

    // Test default implementations for new trait methods added in issue #525.
    // provider_display_name defaults to name(); fetch_model_list defaults to ModelNotFound.
    // All concrete providers (MiniMax, GLM, OpenAI, Anthropic, Stub, Fake) use these
    // defaults, so we verify the defaults via StubProvider.

    #[tokio::test]
    async fn test_provider_display_name_default() {
        let provider = StubProvider::new();
        // Default impl of provider_display_name returns self.name()
        assert_eq!(provider.provider_display_name(), provider.name());
    }

    #[tokio::test]
    async fn test_fetch_model_list_default_returns_model_not_found() {
        let provider = StubProvider::new();
        let result: Result<Vec<crate::llm::ModelInfo>, LLMError> =
            provider.fetch_model_list("fake-token").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, LLMError::ModelNotFound(_)));
    }
}
