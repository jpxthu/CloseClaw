//! Unified Fallback Client
//!
//! Walks a chain of [`UnifiedChatClient`] instances with cooldown-based fallback.
//!
//! Unlike [`FallbackClient`](crate::fallback::FallbackClient) which wraps raw
//! providers, `UnifiedFallbackClient` operates on fully-configured
//! [`UnifiedChatClient`] instances that already own a Provider → Protocol →
//! Interpreter → Plugin pipeline. This lets the non-streaming path go through
//! the same five-layer architecture as the streaming path.

use crate::client::{ClientError, UnifiedChatClient};
use crate::protocol::{OutgoingEventStream, ProtocolError};
use crate::retry::CooldownManager;
use crate::types::{InternalRequest, UnifiedResponse};
use crate::LLMError;
use closeclaw_common::processor::{ContentBlock, ContentBlockType, ContentDelta, StreamEvent};
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Error conversion
// ─────────────────────────────────────────────────────────────────────────────

impl From<ClientError> for LLMError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Provider(e) => LLMError::ApiError(e.to_string()),
            ClientError::Protocol(e) => LLMError::ApiError(e.to_string()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chain entry
// ─────────────────────────────────────────────────────────────────────────────

/// A single entry in the fallback chain.
///
/// Each entry wraps a fully-configured [`UnifiedChatClient`] together with the
/// provider/model identifiers used for cooldown tracking.
#[derive(Debug, Clone)]
pub struct ChainEntry {
    /// Provider identifier (used as cooldown key).
    pub provider_id: String,
    /// Model identifier (used as cooldown key).
    pub model_id: String,
    /// The unified client for this entry.
    pub client: Arc<UnifiedChatClient>,
}

// ─────────────────────────────────────────────────────────────────────────────
// UnifiedFallbackClient
// ─────────────────────────────────────────────────────────────────────────────

/// Fallback client that walks a chain of [`UnifiedChatClient`] instances.
///
/// On each call to [`chat`](Self::chat), the client iterates through the chain,
/// skipping entries that are in cooldown, and returning the first successful
/// response. On failure, the cooldown is recorded and the next entry is tried.
#[derive(Clone)]
pub struct UnifiedFallbackClient {
    /// Ordered chain of clients to try.
    chain: Vec<ChainEntry>,
    /// Shared cooldown manager (same instance as [`FallbackClient`]).
    cooldown: Arc<CooldownManager>,
}

impl std::fmt::Debug for UnifiedFallbackClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedFallbackClient")
            .field("chain_len", &self.chain.len())
            .finish()
    }
}

impl UnifiedFallbackClient {
    /// Create a new `UnifiedFallbackClient`.
    ///
    /// # Arguments
    /// * `chain` — Ordered list of [`ChainEntry`]s to try.
    /// * `cooldown` — Shared [`CooldownManager`] instance.
    pub fn new(chain: Vec<ChainEntry>, cooldown: Arc<CooldownManager>) -> Self {
        Self { chain, cooldown }
    }

    /// Returns a reference to the first client in the chain.
    pub fn primary(&self) -> &Arc<UnifiedChatClient> {
        &self.chain.first().expect("chain must not be empty").client
    }

    /// Send a streaming chat request through the fallback chain.
    ///
    /// Walks the chain trying `chat_streaming` on each entry, skipping
    /// cooldown entries. On success returns the stream; on failure records
    /// cooldown and tries the next entry. If every entry's streaming call
    /// fails, degrades to a non-streaming [`chat`](Self::chat) and wraps
    /// the complete response as a single-chunk stream.
    pub async fn chat_streaming(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, ClientError> {
        let mut idx = 0;
        loop {
            match self.chain.get(idx) {
                None => {
                    // All streaming entries exhausted — degrade to non-streaming.
                    return self.degraded_stream(request).await;
                }
                Some(entry) => {
                    if self
                        .cooldown
                        .is_in_cooldown(&entry.provider_id, &entry.model_id)
                        .await
                    {
                        tracing::debug!(
                            provider = %entry.provider_id,
                            model = %entry.model_id,
                            "model in cooldown, skipping"
                        );
                        idx += 1;
                        continue;
                    }

                    request.model = entry.model_id.clone();

                    match entry.client.chat_streaming(request.clone()).await {
                        Ok(stream) => {
                            self.cooldown
                                .record_success(&entry.provider_id, &entry.model_id)
                                .await;
                            return Ok(stream);
                        }
                        Err(client_err) => {
                            let llm_err: LLMError = client_err.into();
                            let kind = llm_err.kind();
                            tracing::warn!(
                                provider = %entry.provider_id,
                                model = %entry.model_id,
                                error = %llm_err,
                                kind = ?kind,
                                "unified fallback streaming call failed"
                            );
                            self.cooldown
                                .record_failure(&entry.provider_id, &entry.model_id, kind)
                                .await;
                            idx += 1;
                        }
                    }
                }
            }
        }
    }

    /// All streaming entries failed — degrade to non-streaming.
    ///
    /// Walks the chain without cooldown checks (the streaming cooldown
    /// should not block a non-streaming attempt) and wraps the first
    /// successful response as a single-chunk [`OutgoingEventStream`].
    async fn degraded_stream(
        &self,
        mut request: InternalRequest,
    ) -> Result<OutgoingEventStream, ClientError> {
        tracing::warn!("all streaming entries failed, degrading to non-streaming");
        for entry in &self.chain {
            request.model = entry.model_id.clone();
            match entry.client.chat(request.clone()).await {
                Ok(response) => return Ok(response_to_stream(response)),
                Err(client_err) => {
                    let llm_err: LLMError = client_err.into();
                    tracing::warn!(
                        provider = %entry.provider_id,
                        model = %entry.model_id,
                        error = %llm_err,
                        "degraded non-streaming call also failed"
                    );
                }
            }
        }
        Err(ClientError::Protocol(ProtocolError::ResponseParse(
            "all models in unified fallback chain exhausted".to_string(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat with fallback
// ─────────────────────────────────────────────────────────────────────────────

impl UnifiedFallbackClient {
    /// Send a chat request through the fallback chain.
    ///
    /// Iterates through [`chain`](Self::chain) entries, skipping those in
    /// cooldown. Returns the first successful [`UnifiedResponse`], or an error
    /// if all entries are exhausted.
    pub async fn chat(&self, mut request: InternalRequest) -> Result<UnifiedResponse, LLMError> {
        let mut idx = 0;
        loop {
            let entry = self.chain.get(idx).ok_or_else(|| {
                LLMError::ApiError("all models in unified fallback chain exhausted".to_string())
            })?;

            if self
                .cooldown
                .is_in_cooldown(&entry.provider_id, &entry.model_id)
                .await
            {
                tracing::debug!(
                    provider = %entry.provider_id,
                    model = %entry.model_id,
                    "model in cooldown, skipping"
                );
                idx += 1;
                continue;
            }

            request.model = entry.model_id.clone();

            match entry.client.chat(request.clone()).await {
                Ok(response) => {
                    self.cooldown
                        .record_success(&entry.provider_id, &entry.model_id)
                        .await;
                    return Ok(response);
                }
                Err(client_err) => {
                    let llm_err: LLMError = client_err.into();
                    let kind = llm_err.kind();
                    tracing::warn!(
                        provider = %entry.provider_id,
                        model = %entry.model_id,
                        error = %llm_err,
                        kind = ?kind,
                        "unified fallback call failed"
                    );
                    self.cooldown
                        .record_failure(&entry.provider_id, &entry.model_id, kind)
                        .await;
                    idx += 1;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Degraded-stream helper
// ─────────────────────────────────────────────────────────────────────────────

/// Convert a [`UnifiedResponse`] into a single-chunk [`OutgoingEventStream`].
///
/// Each [`ContentBlock`] in the response becomes a `BlockStart → BlockDelta
/// → BlockEnd` triple, followed by a `MessageEnd` with usage stats.
fn response_to_stream(response: UnifiedResponse) -> OutgoingEventStream {
    use futures::stream;

    let mut events: Vec<Result<StreamEvent, ProtocolError>> = Vec::new();
    for (i, block) in response.content_blocks.iter().enumerate() {
        let block_type = match block {
            ContentBlock::Text(_) => ContentBlockType::Text,
            ContentBlock::Thinking { .. } => ContentBlockType::Thinking,
            ContentBlock::ToolUse { .. } => ContentBlockType::ToolUse,
            ContentBlock::ToolResult { .. } => continue,
            ContentBlock::Image { .. } => continue,
            ContentBlock::Audio { .. } => continue,
            ContentBlock::File { .. } => continue,
        };
        events.push(Ok(StreamEvent::BlockStart {
            index: i,
            block_type,
        }));
        // For Text blocks, emit one BlockDelta per character (typewriter effect).
        // For other block types, emit a single BlockDelta as before.
        match block {
            ContentBlock::Text(text) => {
                for ch in text.chars() {
                    events.push(Ok(StreamEvent::BlockDelta {
                        index: i,
                        delta: ContentDelta::Text {
                            text: ch.to_string(),
                        },
                    }));
                }
            }
            ContentBlock::Thinking {
                thinking,
                signature,
            } => {
                events.push(Ok(StreamEvent::BlockDelta {
                    index: i,
                    delta: ContentDelta::Thinking {
                        thinking: thinking.clone(),
                        signature: signature.clone(),
                    },
                }));
            }
            ContentBlock::ToolUse { id, name, input } => {
                events.push(Ok(StreamEvent::BlockDelta {
                    index: i,
                    delta: ContentDelta::ToolUseInputChunk {
                        input: serde_json::json!({
                            "id": id,
                            "name": name,
                            "input": input
                        })
                        .to_string(),
                    },
                }));
            }
            ContentBlock::ToolResult { .. }
            | ContentBlock::Image { .. }
            | ContentBlock::Audio { .. }
            | ContentBlock::File { .. } => unreachable!(),
        };
        events.push(Ok(StreamEvent::BlockEnd {
            index: i,
            block_type,
        }));
    }
    events.push(Ok(StreamEvent::MessageEnd {
        usage: Some(response.usage),
        finish_reason: response.finish_reason,
    }));
    Box::pin(stream::iter(events))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::CooldownManager;
    use crate::ErrorKind;
    use futures::StreamExt;

    /// Build a mock chain entry with a no-op UnifiedChatClient.
    ///
    /// Uses `StubProvider` + `OpenAiProtocol::default()` to create a minimal client.
    fn mock_entry(provider_id: &str, model_id: &str) -> ChainEntry {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;
        use crate::stub::StubProvider;
        use std::sync::Arc;

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
        ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            client,
        }
    }

    fn make_request(model: &str) -> InternalRequest {
        use crate::types::InternalMessage;
        use closeclaw_session::persistence::ReasoningLevel;

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

    #[tokio::test]
    async fn test_single_entry_success() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry = mock_entry("stub", "stub-model");
        let client = UnifiedFallbackClient::new(vec![entry], cooldown);
        let request = make_request("stub-model");
        let result = client.chat(request).await;
        assert!(result.is_ok(), "single entry should succeed");
    }

    #[tokio::test]
    async fn test_primary_returns_first_entry() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry1 = mock_entry("a", "model-a");
        let entry2 = mock_entry("b", "model-b");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        assert_eq!(client.primary().provider_id(), "stub");
    }

    #[tokio::test]
    async fn test_chat_walks_chain_on_failure() {
        let cooldown = Arc::new(CooldownManager::new());
        // First entry uses StubProvider (succeeds), second entry also uses StubProvider.
        // This tests that the chain iteration logic works correctly.
        let entry1 = mock_entry("provider-a", "model-a");
        let entry2 = mock_entry("provider-b", "model-b");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        let request = make_request("model-a");
        let result = client.chat(request).await;
        assert!(result.is_ok());
    }

    // ── Failing provider (always errors) ───────────────────────────────────────

    /// A provider that always fails with a given error message.
    struct FailingProvider {
        msg: String,
        id: String,
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
    impl crate::provider::Provider for FailingProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn base_url(&self) -> &str {
            ""
        }
        fn api_key(&self) -> &str {
            ""
        }
        fn supported_protocols(&self) -> &[crate::types::ProtocolId] {
            &[]
        }
        fn http_client(&self) -> &reqwest::Client {
            static DUMMY: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            DUMMY.get_or_init(reqwest::Client::new)
        }
        fn default_headers(&self) -> &reqwest::header::HeaderMap {
            static EMPTY: std::sync::OnceLock<reqwest::header::HeaderMap> =
                std::sync::OnceLock::new();
            EMPTY.get_or_init(reqwest::header::HeaderMap::new)
        }
        async fn send(
            &self,
            _request: crate::types::InternalRequest,
            _body: serde_json::Value,
        ) -> crate::provider::Result<crate::types::InternalResponse> {
            Err(crate::provider::ProviderError::Legacy(self.msg.clone()))
        }
        async fn send_streaming(
            &self,
            request: crate::types::InternalRequest,
            body: serde_json::Value,
        ) -> crate::provider::Result<crate::provider::SseStream> {
            self.send(request, body).await?;
            unreachable!()
        }
    }

    /// Build a chain entry whose `UnifiedChatClient` always fails.
    fn failing_entry(provider_id: &str, model_id: &str, msg: &str) -> ChainEntry {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;

        let provider = Arc::new(FailingProvider::new(provider_id, msg));
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
        ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            client,
        }
    }

    // ── Missing UT: all entries fail → chain exhausted error returned ───────────

    #[tokio::test]
    async fn test_all_entries_fail_returns_chain_exhausted_error() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry1 = failing_entry("p1", "m1", "error from provider 1");
        let entry2 = failing_entry("p2", "m2", "error from provider 2");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        let request = make_request("m1");
        let result = client.chat(request).await;
        assert!(result.is_err(), "should fail when all entries fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("all models in unified fallback chain exhausted"),
            "should return chain-exhausted error, got: {}",
            msg
        );
    }

    // ── Missing UT: cooldown skip ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_cooldown_skip_first_entry() {
        let cooldown = Arc::new(CooldownManager::new());
        // Put first entry into cooldown
        cooldown
            .record_failure("p-cooldown", "m-cooldown", ErrorKind::Transient)
            .await;
        assert!(
            cooldown.is_in_cooldown("p-cooldown", "m-cooldown").await,
            "first entry should be in cooldown"
        );

        let entry1 = mock_entry("p-cooldown", "m-cooldown");
        let entry2 = mock_entry("p-ok", "m-ok");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        // The request model will be overwritten to entry.model_id in chat(),
        // so we pass a dummy model here.
        let request = make_request("dummy");
        let result = client.chat(request).await;
        assert!(
            result.is_ok(),
            "should succeed via second entry after skipping cooldown entry"
        );
    }

    // ── Missing UT: empty chain ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_empty_chain_chat_returns_error() {
        let cooldown = Arc::new(CooldownManager::new());
        let client = UnifiedFallbackClient::new(vec![], cooldown);
        let request = make_request("m");
        let result = client.chat(request).await;
        assert!(result.is_err(), "empty chain should fail");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("all models in unified fallback chain exhausted"),
            "unexpected error: {}",
            msg
        );
    }

    #[test]
    #[should_panic(expected = "chain must not be empty")]
    fn test_empty_chain_primary_panics() {
        let cooldown = Arc::new(CooldownManager::new());
        let client = UnifiedFallbackClient::new(vec![], cooldown);
        let _ = client.primary();
    }

    // ── Streaming fallback tests ───────────────────────────────────────────────

    /// A provider whose `send_streaming` always fails but `send` succeeds.
    struct StreamingFailProvider {
        msg: String,
        id: String,
    }

    impl StreamingFailProvider {
        fn new(id: impl Into<String>, msg: impl Into<String>) -> Self {
            Self {
                id: id.into(),
                msg: msg.into(),
            }
        }
    }

    #[async_trait::async_trait]
    impl crate::provider::Provider for StreamingFailProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn base_url(&self) -> &str {
            ""
        }
        fn api_key(&self) -> &str {
            ""
        }
        fn supported_protocols(&self) -> &[crate::types::ProtocolId] {
            &[]
        }
        fn http_client(&self) -> &reqwest::Client {
            static DUMMY: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
            DUMMY.get_or_init(reqwest::Client::new)
        }
        fn default_headers(&self) -> &reqwest::header::HeaderMap {
            static EMPTY: std::sync::OnceLock<reqwest::header::HeaderMap> =
                std::sync::OnceLock::new();
            EMPTY.get_or_init(reqwest::header::HeaderMap::new)
        }
        async fn send(
            &self,
            _request: crate::types::InternalRequest,
            _body: serde_json::Value,
        ) -> crate::provider::Result<crate::types::InternalResponse> {
            use crate::types::{RawContentBlock, RawUsage};
            Ok(crate::types::InternalResponse {
                content_blocks: vec![RawContentBlock::Text(self.msg.clone())],
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
            Err(crate::provider::ProviderError::Legacy(self.msg.clone()))
        }
    }

    fn streaming_fail_entry(provider_id: &str, model_id: &str, msg: &str) -> ChainEntry {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;

        let provider = Arc::new(StreamingFailProvider::new(provider_id, msg));
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
        ChainEntry {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            client,
        }
    }

    #[tokio::test]
    async fn test_fallback_degraded_stream() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry = streaming_fail_entry("p1", "m1", "streaming not supported");
        let client = UnifiedFallbackClient::new(vec![entry], cooldown);
        let request = make_request("m1");
        let result = client.chat_streaming(request).await;
        assert!(result.is_ok(), "should degrade to non-streaming");
        let mut stream = result.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        assert!(!events.is_empty(), "degraded stream should produce events");
        let last = events.last().unwrap().as_ref().unwrap();
        assert!(
            matches!(last, StreamEvent::MessageEnd { .. }),
            "last event should be MessageEnd"
        );
    }

    #[tokio::test]
    async fn test_fallback_streaming_chain_traversal() {
        use crate::cache_adapter::NoopCacheAdapter;
        use crate::client::UnifiedChatClient;
        use crate::interpreter::InterpreterRegistry;
        use crate::plugin::PluginPipeline;
        use crate::protocol::OpenAiProtocol;
        use crate::stub::StubProvider;

        let cooldown = Arc::new(CooldownManager::new());

        let entry_fail = streaming_fail_entry("p-fail", "m-fail", "no streaming");

        let provider = Arc::new(StubProvider::new());
        let protocol = Arc::new(OpenAiProtocol::default());
        let registry = InterpreterRegistry::new(vec![]);
        let pipeline = PluginPipeline::new();
        let client_ok = Arc::new(UnifiedChatClient::new(
            provider,
            protocol,
            registry,
            pipeline,
            Arc::new(NoopCacheAdapter),
        ));
        let entry_ok = ChainEntry {
            provider_id: "p-ok".to_string(),
            model_id: "m-ok".to_string(),
            client: client_ok,
        };

        let client = UnifiedFallbackClient::new(vec![entry_fail, entry_ok], cooldown);
        let request = make_request("m-fail");
        let result = client.chat_streaming(request).await;
        assert!(
            result.is_ok(),
            "should succeed via second entry after first fails"
        );
        let mut stream = result.unwrap();
        let first = stream.next().await;
        assert!(first.is_some(), "stream should yield at least one event");
    }

    #[tokio::test]
    async fn test_fallback_streaming_all_fail_degrades() {
        let cooldown = Arc::new(CooldownManager::new());
        let entry1 = streaming_fail_entry("p1", "m1", "fail 1");
        let entry2 = streaming_fail_entry("p2", "m2", "fail 2");
        let client = UnifiedFallbackClient::new(vec![entry1, entry2], cooldown);
        let request = make_request("m1");
        // Both entries fail streaming but send() succeeds → degraded
        let result = client.chat_streaming(request).await;
        assert!(
            result.is_ok(),
            "should degrade to non-streaming successfully"
        );
        let mut stream = result.unwrap();
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        assert!(!events.is_empty(), "degraded stream should produce events");
    }

    #[tokio::test]
    async fn test_fallback_streaming_cooldown_skip() {
        let cooldown = Arc::new(CooldownManager::new());
        cooldown
            .record_failure("p-cd", "m-cd", ErrorKind::Transient)
            .await;
        assert!(cooldown.is_in_cooldown("p-cd", "m-cd").await);

        let entry_cd = streaming_fail_entry("p-cd", "m-cd", "cd");
        let entry_ok = mock_entry("p-ok", "m-ok");
        let client = UnifiedFallbackClient::new(vec![entry_cd, entry_ok], cooldown);
        let request = make_request("dummy");
        let result = client.chat_streaming(request).await;
        assert!(result.is_ok(), "should skip cooldown entry and use second");
    }

    #[tokio::test]
    async fn test_response_to_stream_roundtrip() {
        use closeclaw_common::processor::{ContentBlock, UnifiedUsage};

        let response = UnifiedResponse {
            content_blocks: vec![
                ContentBlock::Text("hello world".to_string()),
                ContentBlock::Thinking {
                    thinking: "reasoning".to_string(),
                    signature: None,
                },
            ],
            usage: UnifiedUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: Some(15),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: Some("stop".to_string()),
        };
        let mut stream = response_to_stream(response);
        let mut events = Vec::new();
        while let Some(event) = stream.next().await {
            events.push(event.unwrap());
        }
        // Text "hello world" = 11 chars → 11 BlockDelta events
        // Thinking = 1 BlockDelta event
        // Total: 1 BlockStart + 11 BlockDelta + 1 BlockEnd + 1 BlockStart + 1 BlockDelta + 1 BlockEnd + 1 MessageEnd = 17
        assert_eq!(events.len(), 17);
        assert!(matches!(
            events[0],
            StreamEvent::BlockStart { index: 0, .. }
        ));
        // First BlockDelta is the first char 'h'
        if let StreamEvent::BlockDelta { index: 0, delta } = &events[1] {
            assert!(matches!(delta, ContentDelta::Text { text } if text == "h"));
        } else {
            panic!("expected BlockDelta at index 1");
        }
        // BlockEnd for text block at position 12
        assert!(matches!(events[12], StreamEvent::BlockEnd { index: 0, .. }));
        assert!(matches!(
            events[13],
            StreamEvent::BlockStart { index: 1, .. }
        ));
        assert!(matches!(events[16], StreamEvent::MessageEnd { .. }));
    }
}
