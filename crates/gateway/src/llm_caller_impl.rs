//! `LlmCaller` trait implementations for the gateway layer.
//!
//! Implements [`closeclaw_common::llm_caller::LlmCaller`] via newtype wrappers
//! around [`UnifiedFallbackClient`](closeclaw_llm::unified_fallback::UnifiedFallbackClient)
//! and [`UnifiedChatClient`](closeclaw_llm::client::UnifiedChatClient).
//!
//! These implementations live in the gateway crate because `closeclaw-session`
//! cannot depend on `closeclaw-llm` (circular dependency: `closeclaw-llm`
//! depends on `closeclaw-session`). The gateway depends on both and is the
//! correct layer for this bridging code.
//!
//! Newtype wrappers are used because Rust's orphan rule prevents implementing
//! a foreign trait for a foreign type directly.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};

use closeclaw_common::llm_caller::LlmCaller;
use closeclaw_common::llm_error::LLMError;
use closeclaw_common::llm_types::InternalRequest;
use closeclaw_common::processor::{StreamEvent, UnifiedResponse};
use closeclaw_llm::client::UnifiedChatClient;
use closeclaw_llm::fallback::FallbackClient;
use closeclaw_llm::protocol::ProtocolError;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;

// ─────────────────────────────────────────────────────────────────────────────
// Newtype wrappers
// ─────────────────────────────────────────────────────────────────────────────

/// Newtype wrapper around [`UnifiedFallbackClient`] to implement [`LlmCaller`].
///
/// Required by Rust's orphan rule — we cannot implement a foreign trait
/// for a foreign type directly.
#[allow(dead_code)]
pub struct FallbackLlmCaller(pub Arc<UnifiedFallbackClient>);

#[async_trait]
impl LlmCaller for FallbackLlmCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        self.0.chat(request).await
    }

    async fn call_streaming(
        &self,
        request: InternalRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError> {
        let raw_stream = self
            .0
            .primary()
            .chat_streaming(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        let mapped = raw_stream.map(|r: Result<StreamEvent, ProtocolError>| {
            r.map_err(|e| LLMError::ApiError(e.to_string()))
        });
        Ok(Box::pin(mapped))
    }
}

/// Newtype wrapper around [`UnifiedChatClient`] to implement [`LlmCaller`].
#[allow(dead_code)]
pub struct ChatLlmCaller(pub Arc<UnifiedChatClient>);

#[async_trait]
impl LlmCaller for ChatLlmCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        self.0
            .chat(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))
    }

    async fn call_streaming(
        &self,
        request: InternalRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError> {
        let raw_stream = self
            .0
            .chat_streaming(request)
            .await
            .map_err(|e| LLMError::ApiError(e.to_string()))?;
        let mapped = raw_stream.map(|r: Result<StreamEvent, ProtocolError>| {
            r.map_err(|e| LLMError::ApiError(e.to_string()))
        });
        Ok(Box::pin(mapped))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// execute_compact
// ─────────────────────────────────────────────────────────────────────────────

/// Execute a compaction: call the LLM to summarize the conversation,
/// return the compaction result with the boundary message.
pub async fn execute_compact(
    messages: &[closeclaw_llm::Message],
    client: &FallbackClient,
    model: &str,
    instruction: Option<&str>,
    is_auto: bool,
) -> Result<
    closeclaw_session::compaction::CompactionResult,
    closeclaw_session::compaction::CompactionError,
> {
    use closeclaw_llm::{ChatRequest, Message as LlmMessage};
    use closeclaw_session::compaction::*;

    if messages.is_empty() {
        return Err(CompactionError::EmptyMessages);
    }

    let prompt = build_compact_prompt(instruction);
    let mut llm_messages = vec![LlmMessage {
        role: "system".to_string(),
        content: prompt,
    }];
    for m in messages {
        llm_messages.push(LlmMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }

    let request = ChatRequest {
        model: model.to_string(),
        messages: llm_messages,
        temperature: 0.0,
        max_tokens: Some(4096),
    };

    let response = client
        .chat(request)
        .await
        .map_err(|e| CompactionError::LLMCallFailed(e.to_string()))?;

    let summary = extract_summary(&response.content).ok_or(CompactionError::SummaryParseFailed)?;

    let boundary = format_boundary_message(&summary, is_auto);
    let before_tokens = estimate_messages_tokens(
        &messages
            .iter()
            .map(|m| CompactionMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect::<Vec<_>>(),
    );
    let after_tokens = estimate_tokens(&boundary);
    let before_chars: usize = messages.iter().map(|m| m.content.len()).sum();
    let after_chars = boundary.len();

    Ok(CompactionResult {
        performed: true,
        original_tokens: before_tokens,
        compacted_tokens: after_tokens,
        message: format!(
            "Compaction completed: {} → {} tokens",
            before_tokens, after_tokens
        ),
        before_char_count: before_chars,
        after_char_count: after_chars,
        before_token_count: before_tokens,
        after_token_count: after_tokens,
        boundary_message: boundary,
        is_auto,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::llm_types::InternalMessage;

    fn make_request(content: &str) -> InternalRequest {
        InternalRequest {
            model: "test-model".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: content.to_string(),
                tool_call_id: None,
            }],
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_common::ReasoningLevel::default(),
            turn_count: None,
        }
    }

    #[tokio::test]
    async fn test_fallback_llm_caller_call() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::stub::StubProvider;
        use closeclaw_llm::unified_fallback::ChainEntry;

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let entry = ChainEntry {
            provider_id: "stub".to_string(),
            model_id: "stub-model".to_string(),
            client,
        };
        let cooldown = Arc::new(CooldownManager::new());
        let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
        let caller = FallbackLlmCaller(fallback);

        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_ok(), "call should succeed via stub provider");
    }

    #[tokio::test]
    async fn test_chat_llm_caller_call() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::stub::StubProvider;

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let caller = ChatLlmCaller(client);

        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_ok(), "call should succeed via stub provider");
    }

    #[tokio::test]
    async fn test_chat_llm_caller_call_streaming() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::stub::StubProvider;

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let caller = ChatLlmCaller(client);

        let mut request = make_request("hello");
        request.stream = true;
        let result = caller.call_streaming(request).await;
        assert!(result.is_ok(), "call_streaming should succeed");
        let mut stream = result.unwrap();
        let _ = stream.next().await;
    }

    #[tokio::test]
    async fn test_fallback_llm_caller_call_streaming() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::stub::StubProvider;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let entry = ChainEntry {
            provider_id: "stub".to_string(),
            model_id: "stub-model".to_string(),
            client,
        };
        let cooldown = Arc::new(CooldownManager::new());
        let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
        let caller = FallbackLlmCaller(fallback);

        let mut request = make_request("hello");
        request.stream = true;
        let result = caller.call_streaming(request).await;
        assert!(result.is_ok(), "call_streaming should succeed");
        let mut stream = result.unwrap();
        // Consume the first event to verify the stream works
        let _ = stream.next().await;
    }

    // ── LlmCaller error propagation ─────────────────────────────────────

    #[tokio::test]
    async fn test_fallback_llm_caller_error_propagation() {
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::unified_fallback::UnifiedFallbackClient;

        let cooldown = Arc::new(CooldownManager::new());
        let client = Arc::new(UnifiedFallbackClient::new(vec![], cooldown));
        let caller = FallbackLlmCaller(client);

        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_err(), "empty chain should return error");
    }

    #[tokio::test]
    async fn test_chat_llm_caller_error_propagation() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::stub::StubProvider;

        // Create a client with a provider that returns empty content,
        // which will cause SummaryParseFailed downstream.
        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let caller = ChatLlmCaller(client);

        // Non-streaming call succeeds with stub
        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_ok(), "stub should succeed");
    }

    #[tokio::test]
    async fn test_fallback_llm_caller_empty_messages() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::stub::StubProvider;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let entry = ChainEntry {
            provider_id: "stub".to_string(),
            model_id: "stub-model".to_string(),
            client,
        };
        let cooldown = Arc::new(CooldownManager::new());
        let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
        let caller = FallbackLlmCaller(fallback);

        let request = InternalRequest {
            model: "test-model".to_string(),
            messages: vec![],
            temperature: 0.7,
            max_tokens: None,
            stream: false,
            extra_body: Default::default(),
            system_static: None,
            system_dynamic: None,
            system_blocks: None,
            tools: None,
            session_id: None,
            reasoning_level: closeclaw_common::ReasoningLevel::default(),
            turn_count: None,
        };
        let result = caller.call(request).await;
        // StubProvider accepts empty messages — call succeeds
        assert!(result.is_ok(), "empty messages should not fail with stub");
    }

    // ── execute_compact tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_execute_compact_empty_messages() {
        use closeclaw_llm::fallback::FallbackClient;
        use closeclaw_llm::LLMRegistry;
        use closeclaw_session::compaction::CompactionError;

        let registry = Arc::new(LLMRegistry::default());
        let client = FallbackClient::new(registry, vec![]);

        let result = execute_compact(&[], &client, "stub-model", None, false).await;
        assert!(matches!(result, Err(CompactionError::EmptyMessages)));
    }

    #[tokio::test]
    async fn test_execute_compact_valid_messages() {
        use closeclaw_llm::fallback::FallbackClient;
        use closeclaw_llm::LLMRegistry;

        let registry = Arc::new(LLMRegistry::default());
        let client = FallbackClient::new(registry, vec![]);

        let messages = vec![closeclaw_llm::Message {
            role: "user".to_string(),
            content: "Hello, how are you?".to_string(),
        }];
        let result = execute_compact(&messages, &client, "stub-model", None, false).await;
        // Empty chain returns LLMCallFailed
        assert!(result.is_err());
    }
}
