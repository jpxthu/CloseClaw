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
use closeclaw_llm::protocol::ProtocolError;
use closeclaw_llm::session::{InjectionPosition, MemoryInjection};
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;

// ─────────────────────────────────────────────────────────────────────────────
// Helper: memory injection
// ─────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
// These functions and types will be used by session_handler callers in Step 1.5.

/// Convert a [`MemoryInjection`] into a tool-role `InternalMessage`.
#[allow(dead_code)]
pub fn memory_injection_to_message(
    injection: &MemoryInjection,
) -> closeclaw_common::llm_types::InternalMessage {
    closeclaw_common::llm_types::InternalMessage {
        role: "tool".to_string(),
        content: injection.content.clone(),
        tool_call_id: None,
    }
}

/// Inject memory content into the request's message list if an injection
/// payload is provided.
#[allow(dead_code)]
pub fn inject_memory(request: &mut InternalRequest, injection: &MemoryInjection) {
    let tool_msg = memory_injection_to_message(injection);
    match injection.position_mode {
        InjectionPosition::AfterCurrent => {
            request.messages.push(tool_msg);
        }
        InjectionPosition::BeforeNext => {
            request.messages.insert(0, tool_msg);
        }
    }
}

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

    #[test]
    fn test_memory_injection_to_message() {
        let injection = MemoryInjection {
            content: "memory content here".to_string(),
            position_mode: InjectionPosition::AfterCurrent,
            injected_event_ids: std::collections::HashSet::new(),
        };
        let msg = memory_injection_to_message(&injection);
        assert_eq!(msg.role, "tool");
        assert_eq!(msg.content, "memory content here");
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_inject_memory_after_current() {
        let mut request = make_request("user msg");
        let injection = MemoryInjection {
            content: "injected".to_string(),
            position_mode: InjectionPosition::AfterCurrent,
            injected_event_ids: std::collections::HashSet::new(),
        };
        inject_memory(&mut request, &injection);
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, "user");
        assert_eq!(request.messages[1].role, "tool");
        assert_eq!(request.messages[1].content, "injected");
    }

    #[test]
    fn test_inject_memory_before_next() {
        let mut request = make_request("user msg");
        let injection = MemoryInjection {
            content: "injected".to_string(),
            position_mode: InjectionPosition::BeforeNext,
            injected_event_ids: std::collections::HashSet::new(),
        };
        inject_memory(&mut request, &injection);
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, "tool");
        assert_eq!(request.messages[0].content, "injected");
        assert_eq!(request.messages[1].role, "user");
    }
}
