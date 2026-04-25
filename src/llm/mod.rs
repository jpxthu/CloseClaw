//! LLM Interface - Abstract trait for multiple LLM providers

pub mod anthropic;
pub mod fallback;
pub mod minimax;
pub mod openai;
pub mod retry;
pub mod stub;

#[cfg(feature = "fake-llm")]
pub mod fake;
#[cfg(feature = "fake-llm")]
pub use fake::FakeProvider;

pub use anthropic::AnthropicProvider;
pub use minimax::MiniMaxProvider;
pub use openai::OpenAIProvider;
pub use stub::StubProvider;

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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// LLM provider trait - implemented by each LLM provider
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &str;

    /// Send a chat request
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LLMError>;

    /// List available models
    fn models(&self) -> Vec<&str>;

    /// Returns true if this is a stub provider that returns fake responses.
    /// When true, callers should treat this as a configuration error.
    fn is_stub(&self) -> bool {
        false
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
}
