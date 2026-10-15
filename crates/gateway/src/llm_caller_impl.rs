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
use closeclaw_llm::protocol::ProtocolError;
use closeclaw_llm::unified_fallback::UnifiedFallbackClient;

// ─────────────────────────────────────────────────────────────────────────────
// Newtype wrappers
// ─────────────────────────────────────────────────────────────────────────────

/// Newtype wrapper around [`UnifiedFallbackClient`] to implement [`LlmCaller`].
///
/// Required by Rust's orphan rule — we cannot implement a foreign trait
/// for a foreign type directly.
pub struct FallbackLlmCaller(pub Arc<UnifiedFallbackClient>);

#[async_trait]
impl LlmCaller for FallbackLlmCaller {
    async fn call(&self, request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        self.0.chat(request).await
    }

    async fn call_with_target(
        &self,
        request: InternalRequest,
    ) -> Result<(UnifiedResponse, Option<closeclaw_common::CallTarget>), LLMError> {
        self.0.chat_with_target(request).await
    }

    async fn call_streaming(
        &self,
        request: InternalRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>, LLMError> {
        let raw_stream = self
            .0
            .chat_streaming(request)
            .await
            .map_err(LLMError::from)?;
        let mapped = raw_stream.map(|r: Result<StreamEvent, ProtocolError>| {
            r.map_err(|e| LLMError::ApiError(e.to_string()))
        });
        Ok(Box::pin(mapped))
    }

    async fn call_streaming_with_target(
        &self,
        request: InternalRequest,
    ) -> Result<
        (
            Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
            Option<closeclaw_common::CallTarget>,
        ),
        LLMError,
    > {
        let (raw_stream, target) = self
            .0
            .chat_streaming_with_target(request)
            .await
            .map_err(LLMError::from)?;
        let mapped = raw_stream.map(|r: Result<StreamEvent, ProtocolError>| {
            r.map_err(|e| LLMError::ApiError(e.to_string()))
        });
        Ok((Box::pin(mapped), target))
    }

    fn default_header_pairs(&self) -> Vec<(String, String)> {
        self.0.default_header_pairs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_common::llm_types::InternalMessage;
    use closeclaw_llm::UnifiedChatClient;

    /// A provider whose non-streaming `send` always fails.
    struct FailingProvider {
        id: String,
        msg: String,
    }

    impl FailingProvider {
        fn new(id: impl Into<String>, msg: impl Into<String>) -> Self {
            Self {
                id: id.into(),
                msg: msg.into(),
            }
        }
    }

    #[async_trait::async_trait]
    impl closeclaw_llm::provider::Provider for FailingProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn base_url(&self) -> &str {
            ""
        }
        fn api_key(&self) -> &str {
            ""
        }
        fn supported_protocols(&self) -> &[closeclaw_llm::types::ProtocolId] {
            &[]
        }
        fn http_client(&self) -> &reqwest::Client {
            static D: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            D.get_or_init(reqwest::Client::new)
        }
        fn default_headers(&self) -> &reqwest::header::HeaderMap {
            static H: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
            H.get_or_init(reqwest::header::HeaderMap::new)
        }
        async fn send(
            &self,
            _req: closeclaw_llm::types::InternalRequest,
            _body: serde_json::Value,
        ) -> closeclaw_llm::provider::Result<serde_json::Value> {
            Err(closeclaw_llm::provider::ProviderError::Legacy(
                self.msg.clone(),
            ))
        }
        async fn send_streaming(
            &self,
            _req: closeclaw_llm::types::InternalRequest,
            _body: serde_json::Value,
        ) -> closeclaw_llm::provider::Result<closeclaw_llm::provider::SseStream> {
            Err(closeclaw_llm::provider::ProviderError::Legacy(
                "streaming not supported".to_string(),
            ))
        }
    }

    /// A provider whose streaming `send_streaming` always fails while its
    /// non-streaming `send` succeeds.
    struct StreamingFailProvider;

    #[async_trait::async_trait]
    impl closeclaw_llm::provider::Provider for StreamingFailProvider {
        fn id(&self) -> &str {
            "fail"
        }
        fn base_url(&self) -> &str {
            ""
        }
        fn api_key(&self) -> &str {
            ""
        }
        fn supported_protocols(&self) -> &[closeclaw_llm::types::ProtocolId] {
            &[]
        }
        fn http_client(&self) -> &reqwest::Client {
            static D: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            D.get_or_init(reqwest::Client::new)
        }
        fn default_headers(&self) -> &reqwest::header::HeaderMap {
            static H: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
            H.get_or_init(reqwest::header::HeaderMap::new)
        }
        async fn send(
            &self,
            _req: closeclaw_llm::types::InternalRequest,
            _body: serde_json::Value,
        ) -> closeclaw_llm::provider::Result<serde_json::Value> {
            use closeclaw_llm::types::{RawContentBlock, RawUsage};
            let resp = closeclaw_llm::types::InternalResponse {
                content_blocks: vec![RawContentBlock::Text("ok".to_string())],
                usage: RawUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: Some(0),
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                },
                finish_reason: None,
            };
            Ok(serde_json::to_value(&resp).unwrap())
        }
        async fn send_streaming(
            &self,
            _req: closeclaw_llm::types::InternalRequest,
            _body: serde_json::Value,
        ) -> closeclaw_llm::provider::Result<closeclaw_llm::provider::SseStream> {
            Err(closeclaw_llm::provider::ProviderError::Legacy(
                "streaming not supported".to_string(),
            ))
        }
    }

    fn make_request(content: &str) -> InternalRequest {
        InternalRequest {
            model: "test-model".to_string(),
            messages: vec![InternalMessage {
                role: "user".to_string(),
                content: content.to_string(),
                content_blocks: None,
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
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
        let caller = FallbackLlmCaller(fallback);

        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_ok(), "call should succeed via stub provider");
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
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
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

    /// Verify that `call_streaming` walks the fallback chain: first entry
    /// streaming fails → falls through to second entry which succeeds.
    #[tokio::test]
    async fn test_fallback_llm_caller_streaming_chain_traversal() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::stub::StubProvider;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        // First entry: streaming always fails, but non-streaming works.
        struct StreamingFailProvider;

        #[async_trait::async_trait]
        impl closeclaw_llm::provider::Provider for StreamingFailProvider {
            fn id(&self) -> &str {
                "fail"
            }
            fn base_url(&self) -> &str {
                ""
            }
            fn api_key(&self) -> &str {
                ""
            }
            fn supported_protocols(&self) -> &[closeclaw_llm::types::ProtocolId] {
                &[]
            }
            fn http_client(&self) -> &reqwest::Client {
                static D: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
                D.get_or_init(reqwest::Client::new)
            }
            fn default_headers(&self) -> &reqwest::header::HeaderMap {
                static H: std::sync::OnceLock<reqwest::header::HeaderMap> =
                    std::sync::OnceLock::new();
                H.get_or_init(reqwest::header::HeaderMap::new)
            }
            async fn send(
                &self,
                _req: closeclaw_llm::types::InternalRequest,
                _body: serde_json::Value,
            ) -> closeclaw_llm::provider::Result<serde_json::Value> {
                use closeclaw_llm::types::{RawContentBlock, RawUsage};
                let resp = closeclaw_llm::types::InternalResponse {
                    content_blocks: vec![RawContentBlock::Text("ok".to_string())],
                    usage: RawUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: Some(0),
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    },
                    finish_reason: None,
                };
                Ok(serde_json::to_value(&resp).unwrap())
            }
            async fn send_streaming(
                &self,
                _req: closeclaw_llm::types::InternalRequest,
                _body: serde_json::Value,
            ) -> closeclaw_llm::provider::Result<closeclaw_llm::provider::SseStream> {
                Err(closeclaw_llm::provider::ProviderError::Legacy(
                    "streaming not supported".to_string(),
                ))
            }
        }

        let fail_client = Arc::new(UnifiedChatClient::new(
            Arc::new(StreamingFailProvider),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let entry_fail = ChainEntry {
            provider_id: "fail".to_string(),
            model_id: "fail-model".to_string(),
            client: fail_client,
        };

        // Second entry: streaming works (StubProvider)
        let ok_client = Arc::new(UnifiedChatClient::new(
            Arc::new(StubProvider::new()),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let entry_ok = ChainEntry {
            provider_id: "stub".to_string(),
            model_id: "stub-model".to_string(),
            client: ok_client,
        };

        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let fallback = Arc::new(UnifiedFallbackClient::new(
            vec![entry_fail, entry_ok],
            cooldown,
        ));
        let caller = FallbackLlmCaller(fallback);

        let mut request = make_request("hello");
        request.stream = true;
        let result = caller.call_streaming(request).await;
        assert!(
            result.is_ok(),
            "call_streaming should succeed via second entry"
        );
        let mut stream = result.unwrap();
        let first = stream.next().await;
        assert!(first.is_some(), "stream should yield at least one event");
    }

    // ── LlmCaller error propagation ─────────────────────────────────────

    #[tokio::test]
    async fn test_fallback_llm_caller_error_propagation() {
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::unified_fallback::UnifiedFallbackClient;

        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let client = Arc::new(UnifiedFallbackClient::new(vec![], cooldown));
        let caller = FallbackLlmCaller(client);

        let request = make_request("hello");
        let result = caller.call(request).await;
        assert!(result.is_err(), "empty chain should return error");
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
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
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

    /// Verify that `call_streaming` degrades to non-streaming when all
    /// entries' streaming fails but non-streaming succeeds.
    #[tokio::test]
    async fn test_fallback_llm_caller_call_streaming_degraded() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        // Provider that fails on streaming but succeeds on non-streaming.
        struct StreamingFailProvider;

        #[async_trait::async_trait]
        impl closeclaw_llm::provider::Provider for StreamingFailProvider {
            fn id(&self) -> &str {
                "fail"
            }
            fn base_url(&self) -> &str {
                ""
            }
            fn api_key(&self) -> &str {
                ""
            }
            fn supported_protocols(&self) -> &[closeclaw_llm::types::ProtocolId] {
                &[]
            }
            fn http_client(&self) -> &reqwest::Client {
                static D: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
                D.get_or_init(reqwest::Client::new)
            }
            fn default_headers(&self) -> &reqwest::header::HeaderMap {
                static H: std::sync::OnceLock<reqwest::header::HeaderMap> =
                    std::sync::OnceLock::new();
                H.get_or_init(reqwest::header::HeaderMap::new)
            }
            async fn send(
                &self,
                _req: closeclaw_llm::types::InternalRequest,
                _body: serde_json::Value,
            ) -> closeclaw_llm::provider::Result<serde_json::Value> {
                use closeclaw_llm::types::{RawContentBlock, RawUsage};
                let resp = closeclaw_llm::types::InternalResponse {
                    content_blocks: vec![RawContentBlock::Text("ok".to_string())],
                    usage: RawUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: Some(0),
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        reasoning_tokens: None,
                    },
                    finish_reason: None,
                };
                Ok(serde_json::to_value(&resp).unwrap())
            }
            async fn send_streaming(
                &self,
                _req: closeclaw_llm::types::InternalRequest,
                _body: serde_json::Value,
            ) -> closeclaw_llm::provider::Result<closeclaw_llm::provider::SseStream> {
                Err(closeclaw_llm::provider::ProviderError::Legacy(
                    "streaming not supported".to_string(),
                ))
            }
        }

        let provider = Arc::new(StreamingFailProvider);
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
            provider_id: "fail".to_string(),
            model_id: "fail-model".to_string(),
            client,
        };
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let fallback = Arc::new(UnifiedFallbackClient::new(vec![entry], cooldown));
        let caller = FallbackLlmCaller(fallback);

        let mut request = make_request("hello");
        request.stream = true;
        let result = caller.call_streaming(request).await;
        assert!(
            result.is_ok(),
            "call_streaming should degrade to non-streaming successfully"
        );
        let mut stream = result.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        assert!(!events.is_empty(), "degraded stream should produce events");
    }

    // ── *_with_target extension methods ────────────────────────────────

    /// Production impl (streaming): after a fallback switch the target
    /// returned with the call is the entry that actually served it (not
    /// the first entry).
    #[tokio::test]
    async fn test_fallback_llm_caller_reports_actual_call_target() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::stub::StubProvider;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        // First entry: streaming always fails; second entry (stub) serves.
        let fail_client = Arc::new(UnifiedChatClient::new(
            Arc::new(StreamingFailProvider),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let ok_client = Arc::new(UnifiedChatClient::new(
            Arc::new(StubProvider::new()),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let fallback = Arc::new(UnifiedFallbackClient::new(
            vec![
                ChainEntry {
                    provider_id: "fail".to_string(),
                    model_id: "fail-model".to_string(),
                    client: fail_client,
                },
                ChainEntry {
                    provider_id: "stub".to_string(),
                    model_id: "stub-model".to_string(),
                    client: ok_client,
                },
            ],
            cooldown,
        ));
        let caller = FallbackLlmCaller(fallback);

        let mut request = make_request("hello");
        request.stream = true;
        let (mut stream, target) = caller
            .call_streaming_with_target(request)
            .await
            .expect("call_streaming_with_target should succeed via second entry");
        // The stream still works.
        assert!(
            stream.next().await.is_some(),
            "stream should yield at least one event"
        );
        let target = target.expect("production impl must return the actually-hit target");
        assert_eq!(target.provider_id, "stub");
        assert_eq!(target.model_id, "stub-model");
        assert_eq!(target.id_string(), "stub/stub-model");
    }

    /// Production impl (non-streaming): the target returned with the call
    /// is the entry that actually served it.
    #[tokio::test]
    async fn test_fallback_llm_caller_call_with_target_reports_actual() {
        use closeclaw_llm::cache_adapter::NoopCacheAdapter;
        use closeclaw_llm::interpreter::InterpreterRegistry;
        use closeclaw_llm::plugin::PluginPipeline;
        use closeclaw_llm::protocol::OpenAiProtocol;
        use closeclaw_llm::retry::CooldownManager;
        use closeclaw_llm::unified_fallback::{ChainEntry, UnifiedFallbackClient};

        let fail_client = Arc::new(UnifiedChatClient::new(
            Arc::new(FailingProvider::new("fail", "boom")),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let ok_client = Arc::new(UnifiedChatClient::new(
            Arc::new(closeclaw_llm::stub::StubProvider::new()),
            Arc::new(OpenAiProtocol::default()),
            InterpreterRegistry::new(vec![]),
            PluginPipeline::new(),
            Arc::new(NoopCacheAdapter),
        ));
        let cooldown_dir = tempfile::TempDir::new().expect("create temp dir");
        let cooldown = Arc::new(CooldownManager::with_path(
            cooldown_dir.path().join("llm_cooldowns.json"),
        ));
        let fallback = Arc::new(UnifiedFallbackClient::new(
            vec![
                ChainEntry {
                    provider_id: "fail".to_string(),
                    model_id: "fail-model".to_string(),
                    client: fail_client,
                },
                ChainEntry {
                    provider_id: "stub".to_string(),
                    model_id: "stub-model".to_string(),
                    client: ok_client,
                },
            ],
            cooldown,
        ));
        let caller = FallbackLlmCaller(fallback);

        let (_, target) = caller
            .call_with_target(make_request("hello"))
            .await
            .expect("call_with_target should succeed via second entry");
        let target = target.expect("production impl must return the actually-hit target");
        assert_eq!(target.id_string(), "stub/stub-model");
    }

    /// Fake that relies on the default extension methods (no override).
    struct BareFakeCaller;

    #[async_trait::async_trait]
    impl LlmCaller for BareFakeCaller {
        async fn call(&self, _request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
            Ok(UnifiedResponse {
                content_blocks: vec![],
                usage: Default::default(),
                finish_reason: None,
                retry_attempts: 0,
            })
        }

        async fn call_streaming(
            &self,
            _request: InternalRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<StreamEvent, LLMError>> + Send>>,
            LLMError,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    /// Fake path: the default extension methods delegate to `call` /
    /// `call_streaming` and return `None` as target without panicking —
    /// consumers degrade to session-config semantics.
    #[tokio::test]
    async fn test_default_call_with_target_is_none_without_panic() {
        let fake = BareFakeCaller;
        let (response, target) = fake
            .call_with_target(make_request("hello"))
            .await
            .expect("default call_with_target must delegate to call");
        assert_eq!(response.retry_attempts, 0);
        assert!(
            target.is_none(),
            "default implementation must report no target"
        );

        let (mut stream, target) = fake
            .call_streaming_with_target(make_request("hello"))
            .await
            .expect("default call_streaming_with_target must delegate to call_streaming");
        assert!(
            stream.next().await.is_none(),
            "delegated stream must be the fake's empty stream"
        );
        assert!(
            target.is_none(),
            "default implementation must report no target"
        );
    }
}
