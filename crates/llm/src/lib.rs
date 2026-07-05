//! LLM Interface — provider abstraction and chat types

pub mod anthropic;
pub mod cache_adapter;
pub mod compaction;
pub mod fallback;
#[cfg(test)]
mod fallback_tests;
pub mod glm;
pub mod glm_stream;
pub mod http_client;
pub mod knowledge;
pub mod llm_caller;
pub mod minimax;
pub mod model_cache;
pub mod model_discovery;
#[cfg(test)]
mod model_discovery_tests;
pub mod model_info;
pub mod openai;
pub mod protocol;
pub mod provider;
pub mod retry;
pub mod session;
pub mod session_exec;
pub(crate) mod session_handles;
pub mod session_state;
pub mod sink_updater;
pub mod stats;
pub mod streaming;
pub mod stub;
pub mod turn;
pub mod types;
#[cfg(test)]
mod types_tests;

pub mod deepseek;
pub mod mimo;
pub mod volcengine;

pub mod client;
#[cfg(test)]
mod client_test;
pub mod interpreter;
#[cfg(test)]
mod interpreter_test;
pub mod plugin;
pub mod unified_fallback;

#[cfg(feature = "fake-llm")]
pub mod fake;
#[cfg(feature = "fake-llm")]
pub use fake::FakeProvider;

pub use anthropic::AnthropicProvider;
pub use deepseek::DeepSeekProvider;
pub use glm::GlmPlugin;
pub use glm::GlmProvider;
pub use http_client::{HttpClient, ReqwestHttpClient};
pub use knowledge::{ModelRecommendParams, ProviderModelKnowledge, ReasoningLevels};
pub use mimo::MimoProvider;
pub use minimax::MiniMaxProvider;
pub use model_cache::{CacheEntry, ModelCache};
pub use model_discovery::ModelDiscovery;
pub use model_info::{DiscoveryResult, DiscoverySource, InputType, ModelInfo};
pub use openai::OpenAIProvider;
pub use protocol::ChatProtocol;
pub use provider::Provider;
pub use volcengine::VolcEngineProvider;

pub use stub::StubProvider;

pub use client::UnifiedChatClient;
pub use interpreter::{DefaultInterpreter, InterpreterRegistry, ModelInterpreter};
pub use plugin::PluginPipeline;
pub use session::{ChatSession, ConversationSession, SessionMessage};
pub use sink_updater::SinkUpdater;
pub use streaming::{StreamDone, StreamingSink};
pub use turn::TurnCounter;
pub use types::{InternalRequest, ProtocolId};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Model discovery — query provider for available model list.
/// Independent from Provider trait (HTTP transport).
#[async_trait]
pub trait ModelLister: Send + Sync {
    /// Fetch available models from provider API.
    async fn fetch_model_list(&self, bearer_token: &str) -> Result<Vec<ModelInfo>, LLMError>;
}

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

// Re-export LLMError and ErrorKind from closeclaw-common (Layer 0)
// for backward compatibility.
pub use closeclaw_common::{ErrorKind, LLMError};

/// LLM Registry - manages multiple providers
pub struct LLMRegistry {
    providers: tokio::sync::RwLock<std::collections::HashMap<String, Arc<dyn Provider>>>,
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

    pub async fn register(&self, name: String, provider: Arc<dyn Provider>) {
        let mut providers = self.providers.write().await;
        providers.insert(name, provider);
    }

    pub async fn get(&self, name: &str) -> Option<Arc<dyn Provider>> {
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
    use crate::stub::StubProvider;

    fn stub_provider() -> Arc<dyn Provider> {
        Arc::new(StubProvider::new())
    }

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

    #[tokio::test]
    async fn test_registry_register_and_retrieve() {
        let registry = LLMRegistry::new();
        let provider = stub_provider();

        registry
            .register("test-stub".to_string(), provider.clone())
            .await;

        let retrieved = registry.get("test-stub").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id(), "stub");
    }

    #[tokio::test]
    async fn test_registry_list() {
        let registry = LLMRegistry::new();
        registry.register("a".to_string(), stub_provider()).await;
        registry.register("b".to_string(), stub_provider()).await;

        let providers = registry.list().await;
        assert_eq!(providers.len(), 2);
        assert!(providers.contains(&"a".to_string()));
        assert!(providers.contains(&"b".to_string()));
    }
}
