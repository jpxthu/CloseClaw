//! Tests for the LLM unified chat client.
//!
//! These tests live here rather than in-client to keep `client.rs` under the
//! 500-line limit imposed by the project style guide.

use futures::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::interpreter::InterpreterRegistry;
use crate::plugin::PluginPipeline;
use crate::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use crate::provider::{Provider, SseStream};
use crate::types::{
    ContentBlock, ContentBlockType, ContentDelta, InternalMessage, InternalRequest,
    InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage, StreamEvent,
    UnifiedResponse, UnifiedUsage,
};
use closeclaw_session::persistence::ReasoningLevel;

use crate::client::UnifiedChatClient;

// ── Stub provider ────────────────────────────────────────────────────────────

struct StubProvider {
    id: &'static str,
    protocol_id: ProtocolId,
}

impl StubProvider {
    fn new() -> Self {
        Self {
            id: "stub",
            protocol_id: ProtocolId::new("stub"),
        }
    }
}

#[async_trait]
impl Provider for StubProvider {
    fn id(&self) -> &str {
        self.id
    }
    fn base_url(&self) -> &str {
        "http://stub"
    }
    fn api_key(&self) -> &str {
        "stub-key"
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        std::slice::from_ref(&self.protocol_id)
    }
    fn http_client(&self) -> &reqwest::Client {
        unreachable!()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        static EMPTY: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
        EMPTY.get_or_init(reqwest::header::HeaderMap::new)
    }

    async fn send(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": "hello from stub" },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 2,
                "total_tokens": 3
            }
        }))
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<SseStream> {
        let (tx, rx) = mpsc::channel(8);
        tx.send(RawSseChunk {
            event_type: "message".into(),
            data: r#"{"choices":[{"delta":{"content":"hi"}}]}"#.into(),
        })
        .await
        .unwrap();
        drop(tx);
        Ok(rx)
    }
}

// ── Stub protocol ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct StubProtocol {
    id: ProtocolId,
}

impl StubProtocol {
    fn new() -> Self {
        Self {
            id: ProtocolId::new("stub"),
        }
    }
}

#[async_trait]
impl ChatProtocol for StubProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        "/chat"
    }

    fn build_request(
        &self,
        _request: &InternalRequest,
    ) -> crate::protocol::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    fn parse_response(&self, body: serde_json::Value) -> crate::protocol::Result<InternalResponse> {
        // Parse OpenAI-style JSON response for testing
        let content = body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let usage = &body["usage"];
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text(content)],
            usage: RawUsage {
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: usage["total_tokens"].as_u64().map(|v| v as u32),
                cache_read_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            },
            finish_reason: body["choices"][0]["finish_reason"]
                .as_str()
                .map(String::from),
        })
    }
    fn decorate_headers(
        &self,
        _headers: &mut reqwest::header::HeaderMap,
    ) -> crate::protocol::Result<()> {
        Ok(())
    }
    fn create_sse_machine(&self) -> crate::types::SseStateMachine {
        crate::types::SseStateMachine::new()
    }

    async fn parse_sse_stream(
        &self,
        incoming: IncomingSseStream,
        _machine: crate::types::SseStateMachine,
    ) -> OutgoingEventStream {
        Box::pin(async_stream::try_stream! {
            let mut stream = incoming;
            while let Some(_chunk) = stream.next().await {
                yield StreamEvent::BlockStart { index: 0, block_type: ContentBlockType::Text };
                yield StreamEvent::BlockDelta { index: 0, delta: ContentDelta::Text { text: "hello".into() } };
                yield StreamEvent::MessageEnd {
                    usage: Some(UnifiedUsage { prompt_tokens: 1, completion_tokens: 1, total_tokens: Some(2), reasoning_tokens: None, cache_read_tokens: None, cache_write_tokens: None }),
                    finish_reason: Some("stop".into()),
                };
            }
        })
    }
}

// ── Counting plugin ───────────────────────────────────────────────────────────

struct CountingPlugin {
    before: Arc<AtomicUsize>,
    after: Arc<AtomicUsize>,
}

impl crate::plugin::ModelPlugin for CountingPlugin {
    fn name(&self) -> &str {
        "counter"
    }
    fn before_request(&self, _r: &mut InternalRequest) {
        self.before.fetch_add(1, Ordering::Relaxed);
    }
    fn after_response(&self, _r: &mut UnifiedResponse) {
        self.after.fetch_add(1, Ordering::Relaxed);
    }
    fn on_stream_event(&self, e: &StreamEvent) -> Option<StreamEvent> {
        Some(e.clone())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_request() -> InternalRequest {
    InternalRequest {
        model: "test-model".to_string(),
        messages: vec![InternalMessage {
            role: "user".into(),
            content: "hello".into(),
            ..Default::default()
        }],
        temperature: 0.0,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
        system_static: None,
        system_dynamic: None,
        system_blocks: None,
        tools: None,
        session_id: None,
        reasoning_level: ReasoningLevel::default(),
        turn_count: None,
    }
}

fn make_client(pipeline: PluginPipeline) -> UnifiedChatClient {
    UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(StubProvider::new()),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        pipeline,
    )
}

// ── Non-streaming tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_full_pipeline() {
    let before = Arc::new(AtomicUsize::new(0));
    let after = Arc::new(AtomicUsize::new(0));
    let client = make_client(PluginPipeline::new().add(Box::new(CountingPlugin {
        before: before.clone(),
        after: after.clone(),
    })));
    let response = client.chat(make_request()).await.unwrap();
    assert_eq!(response.content_blocks.len(), 1);
    assert!(matches!(&response.content_blocks[0], ContentBlock::Text(s) if s == "hello from stub"));
    assert_eq!(before.load(Ordering::Relaxed), 1);
    assert_eq!(after.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn test_chat_empty_pipeline() {
    let client = make_client(PluginPipeline::new());
    let result = client.chat(make_request()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_chat_interpreter_resolves() {
    struct CheckInterpreter;
    impl crate::interpreter::ModelInterpreter for CheckInterpreter {
        fn name(&self) -> &str {
            "check"
        }
        fn interpret_response(&self, _: InternalResponse) -> UnifiedResponse {
            UnifiedResponse {
                content_blocks: vec![ContentBlock::Text("interpreter-ran".into())],
                usage: UnifiedUsage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: Some(0),
                    reasoning_tokens: None,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                },
                finish_reason: None,
                retry_attempts: 0,
            }
        }
        fn interpret_stream_event(&self, e: StreamEvent) -> Option<StreamEvent> {
            Some(e)
        }
    }
    let registry = InterpreterRegistry::new(vec![(Box::new(CheckInterpreter), "stub/*")]);
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(StubProvider::new()),
        Arc::new(StubProtocol::new()),
        registry,
        PluginPipeline::new(),
    );
    let response = client.chat(make_request()).await.unwrap();
    assert!(matches!(&response.content_blocks[0], ContentBlock::Text(s) if s == "interpreter-ran"));
}

#[tokio::test]
async fn test_chat_after_response_mutates() {
    let captured: Arc<Mutex<Option<UnifiedResponse>>> = Arc::new(Mutex::new(None));
    struct CapturePlugin(Arc<Mutex<Option<UnifiedResponse>>>);
    impl crate::plugin::ModelPlugin for CapturePlugin {
        fn name(&self) -> &str {
            "capture"
        }
        fn after_response(&self, resp: &mut UnifiedResponse) {
            *self.0.lock().unwrap() = Some(resp.clone());
        }
    }
    let client = make_client(PluginPipeline::new().add(Box::new(CapturePlugin(captured.clone()))));
    client.chat(make_request()).await.unwrap();
    assert!(captured.lock().unwrap().is_some());
}

// ── Streaming tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_streaming_returns_events() {
    let client = make_client(PluginPipeline::new());
    let stream = client.chat_streaming(make_request()).await.unwrap();
    let events: Vec<_> = stream.collect().await;
    assert!(!events.is_empty());
    assert!(matches!(
        events.last(),
        Some(Ok(StreamEvent::MessageEnd { .. }))
    ));
}

#[tokio::test]
async fn test_chat_streaming_empty_pipeline() {
    let client = make_client(PluginPipeline::new());
    let result = client.chat_streaming(make_request()).await;
    assert!(result.is_ok());
}

// ═══════════════════════════════════════════════════════════════════════════
// Gap 2: default_header_pairs tests
// ═══════════════════════════════════════════════════════════════════════════

/// Provider with non-empty default headers for testing.
struct HeadersProvider {
    headers: reqwest::header::HeaderMap,
}

impl HeadersProvider {
    fn new(headers: reqwest::header::HeaderMap) -> Self {
        Self { headers }
    }
}

#[async_trait]
impl Provider for HeadersProvider {
    fn id(&self) -> &str {
        "headers-test"
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
        unreachable!()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        &self.headers
    }
    async fn send(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<serde_json::Value> {
        Ok(serde_json::json!({"choices": [], "usage": {}}))
    }
    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<SseStream> {
        let (tx, rx) = mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }
}

/// Verify that `default_header_pairs` returns the provider's headers
/// as sorted key-value pairs.
#[test]
fn test_default_header_pairs_returns_provider_headers() {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-custom"),
        HeaderValue::from_static("value1"),
    );
    headers.insert(
        HeaderName::from_static("accept"),
        HeaderValue::from_static("application/json"),
    );

    let provider = HeadersProvider::new(headers);
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(provider),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let pairs = client.default_header_pairs();
    assert_eq!(pairs.len(), 2, "should have 2 header pairs");

    // Pairs are sorted by key.
    assert_eq!(pairs[0].0, "accept");
    assert_eq!(pairs[0].1, "application/json");
    assert_eq!(pairs[1].0, "x-custom");
    assert_eq!(pairs[1].1, "value1");
}

/// Verify that sensitive headers (Authorization, api-key, etc.) have
/// their values replaced with `<redacted>` to avoid leaking credentials.
#[test]
fn test_default_header_pairs_redacts_sensitive_values() {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer sk-secret-token"),
    );
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_static("my-api-key-123"),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("api-key"),
        HeaderValue::from_static("another-secret"),
    );

    let provider = HeadersProvider::new(headers);
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(provider),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let pairs = client.default_header_pairs();
    assert_eq!(pairs.len(), 4, "should have 4 header pairs");

    // Sensitive values are redacted.
    for (key, val) in &pairs {
        match key.as_str() {
            "authorization" | "api-key" | "x-api-key" => {
                assert_eq!(val, "<redacted>", "{} should be redacted", key);
            }
            "content-type" => {
                assert_eq!(val, "application/json");
            }
            _ => {}
        }
    }
}

/// Verify that header pairs are sorted stably — same input always
/// produces the same order, ensuring fingerprint hash consistency.
#[test]
fn test_default_header_pairs_sorted_stably() {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("z-last"),
        HeaderValue::from_static("z"),
    );
    headers.insert(
        HeaderName::from_static("a-first"),
        HeaderValue::from_static("a"),
    );
    headers.insert(
        HeaderName::from_static("m-middle"),
        HeaderValue::from_static("m"),
    );

    let provider = HeadersProvider::new(headers);
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(provider),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let pairs1 = client.default_header_pairs();
    let pairs2 = client.default_header_pairs();

    assert_eq!(pairs1, pairs2, "consecutive calls should return same order");
    assert_eq!(pairs1[0].0, "a-first");
    assert_eq!(pairs1[1].0, "m-middle");
    assert_eq!(pairs1[2].0, "z-last");
}

/// Verify that an empty HeaderMap produces an empty Vec.
#[test]
fn test_default_header_pairs_empty_headers() {
    let provider = HeadersProvider::new(reqwest::header::HeaderMap::new());
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(provider),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let pairs = client.default_header_pairs();
    assert!(pairs.is_empty(), "empty headers should return empty Vec");
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.5: Non-streaming pipeline — Protocol::parse_response integration
// ═══════════════════════════════════════════════════════════════════════════

/// Verify that `chat()` invokes `Protocol::parse_response` and returns
/// the parsed content blocks. Uses a real `OpenAiProtocol` with a stub
/// provider to prove the full `send → parse_response → interpret_response`
/// chain works end-to-end.
#[tokio::test]
async fn test_chat_non_streaming_uses_protocol_parse_response() {
    use crate::protocol::OpenAiProtocol;

    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(StubProvider::new()),
        Arc::new(OpenAiProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let response = client.chat(make_request()).await.unwrap();

    // StubProvider::send returns raw JSON:
    // {"choices":[{"message":{"role":"assistant","content":"hello from stub"},...}],...}
    // OpenAiProtocol::parse_response should parse this into a Text block.
    assert_eq!(response.content_blocks.len(), 1);
    assert!(
        matches!(&response.content_blocks[0], ContentBlock::Text(s) if s == "hello from stub"),
        "Protocol::parse_response should have parsed the raw JSON into a Text block"
    );
}

/// Verify that `Provider::send` returns valid JSON and `Protocol::parse_response`
/// correctly parses it — using `StubProtocol` with `StubProvider`.
#[tokio::test]
async fn test_provider_send_json_parsed_by_protocol() {
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(StubProvider::new()),
        Arc::new(StubProtocol::new()),
        InterpreterRegistry::default(),
        PluginPipeline::new(),
    );

    let response = client.chat(make_request()).await.unwrap();
    // StubProtocol::parse_response extracts "choices[0].message.content"
    assert_eq!(response.content_blocks.len(), 1);
    assert!(
        matches!(&response.content_blocks[0], ContentBlock::Text(s) if s == "hello from stub"),
        "StubProtocol should parse the raw JSON from StubProvider"
    );
    // Verify usage is parsed correctly
    assert_eq!(response.usage.prompt_tokens, 1);
    assert_eq!(response.usage.completion_tokens, 2);
    assert_eq!(response.usage.total_tokens, Some(3));
}

// ═══════════════════════════════════════════════════════════════════════════
// Step 1.3: Anthropic pipeline integration tests
// ═══════════════════════════════════════════════════════════════════════════

use crate::interpreter::AnthropicInterpreter;
use crate::protocol::AnthropicProtocol;
use crate::types::SystemBlock;

/// Anthropic-format stub provider that returns Anthropic-style responses
/// (content array with type field, not OpenAI choices format).
struct AnthropicStubProvider {
    protocol_id: ProtocolId,
}

impl AnthropicStubProvider {
    fn new() -> Self {
        Self {
            protocol_id: ProtocolId::new("anthropic"),
        }
    }
}

#[async_trait]
impl Provider for AnthropicStubProvider {
    fn id(&self) -> &str {
        "anthropic-stub"
    }
    fn base_url(&self) -> &str {
        "http://stub"
    }
    fn api_key(&self) -> &str {
        "stub-key"
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        std::slice::from_ref(&self.protocol_id)
    }
    fn http_client(&self) -> &reqwest::Client {
        unreachable!()
    }
    fn default_headers(&self) -> &reqwest::header::HeaderMap {
        static EMPTY: std::sync::OnceLock<reqwest::header::HeaderMap> = std::sync::OnceLock::new();
        EMPTY.get_or_init(reqwest::header::HeaderMap::new)
    }

    /// Returns Anthropic-format JSON response (content[] array, stop_reason).
    async fn send(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<serde_json::Value> {
        Ok(serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Hello from Anthropic stub"
            }],
            "model": "claude-3-5-sonnet-20241022",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_read_input_tokens": 80,
                "cache_creation_input_tokens": 20
            }
        }))
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::provider::Result<SseStream> {
        let (tx, rx) = mpsc::channel(8);
        // Anthropic SSE format: message_start → content_block_start → delta → block_stop → message_delta → message_stop
        let _ = tx
            .send(RawSseChunk {
                event_type: "message_start".into(),
                data: r#"{"message":{"usage":{"input_tokens":10,"output_tokens":0}}}"#.into(),
            })
            .await;
        let _ = tx
            .send(RawSseChunk {
                event_type: "content_block_start".into(),
                data: r#"{"index":0,"content_block":{"type":"text"}}"#.into(),
            })
            .await;
        let _ = tx
            .send(RawSseChunk {
                event_type: "content_block_delta".into(),
                data: r#"{"index":0,"delta":{"type":"text_delta","text":"hi"}}"#.into(),
            })
            .await;
        let _ = tx
            .send(RawSseChunk {
                event_type: "content_block_stop".into(),
                data: r#"{"index":0}"#.into(),
            })
            .await;
        let _ = tx
            .send(RawSseChunk {
                event_type: "message_delta".into(),
                data: r#"{"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#.into(),
            })
            .await;
        let _ = tx
            .send(RawSseChunk {
                event_type: "message_stop".into(),
                data: "{}".into(),
            })
            .await;
        drop(tx);
        Ok(rx)
    }
}

/// Full pipeline test: AnthropicProtocol + AnthropicInterpreter.
/// Verifies that the response body is parsed correctly as Anthropic format
/// (content[] array with type field) and the interpreter passes it through.
#[tokio::test]
async fn test_anthropic_full_pipeline_parses_response() {
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(AnthropicStubProvider::new()),
        Arc::new(AnthropicProtocol::new()),
        InterpreterRegistry::new(vec![(Box::new(AnthropicInterpreter), "anthropic/*")]),
        PluginPipeline::new(),
    );
    let response = client.chat(make_request()).await.unwrap();
    assert_eq!(response.content_blocks.len(), 1);
    assert!(
        matches!(&response.content_blocks[0], ContentBlock::Text(s) if s == "Hello from Anthropic stub"),
        "expected Text block from Anthropic response, got {:?}",
        response.content_blocks[0]
    );
    // Usage from Anthropic format (input_tokens → prompt_tokens)
    assert_eq!(response.usage.prompt_tokens, 10);
    assert_eq!(response.usage.completion_tokens, 5);
    // Cache usage fields from Anthropic response
    assert_eq!(response.usage.cache_read_tokens, Some(80));
    assert_eq!(response.usage.cache_write_tokens, Some(20));
}

/// Verify that the request body built by AnthropicProtocol follows
/// Anthropic Messages API format (not OpenAI format).
/// Key differences: model/messages/max_tokens at top level,
/// no "choices", content[] array with type field.
#[tokio::test]
async fn test_anthropic_pipeline_request_body_is_anthropic_format() {
    use crate::protocol::ChatProtocol as _;

    let proto = AnthropicProtocol::new();
    let mut req = make_request();
    req.system_blocks = Some(vec![SystemBlock {
        text: "You are helpful.".to_string(),
        cache: true,
    }]);
    let body = proto.build_request(&req).unwrap();

    // Anthropic format: top-level model, messages, max_tokens
    assert!(body.get("model").is_some(), "missing model field");
    assert!(body.get("messages").is_some(), "missing messages field");
    assert!(body.get("max_tokens").is_some(), "missing max_tokens field");

    // NOT OpenAI format: no choices, no temperature at top level in Anthropic style
    assert!(
        body.get("choices").is_none(),
        "should not have OpenAI choices field"
    );

    // System blocks with cache_control
    let system = body.get("system").unwrap().as_array().unwrap();
    assert_eq!(system.len(), 1);
    assert_eq!(system[0]["type"], "text");
    assert_eq!(system[0]["text"], "You are helpful.");
    assert_eq!(
        system[0]["cache_control"],
        serde_json::json!({"type": "ephemeral"}),
        "cache_control marker must be present on system block"
    );
}

/// Verify that messages cache_control markers are on the last message
/// and not on earlier messages (prefix stability).
#[tokio::test]
async fn test_anthropic_pipeline_messages_cache_control_on_last_only() {
    use crate::protocol::ChatProtocol as _;

    let proto = AnthropicProtocol::new();
    let mut req = make_request();
    req.messages = vec![
        InternalMessage {
            role: "user".into(),
            content: "First".into(),
            ..Default::default()
        },
        InternalMessage {
            role: "assistant".into(),
            content: "Second".into(),
            ..Default::default()
        },
        InternalMessage {
            role: "user".into(),
            content: "Third".into(),
            ..Default::default()
        },
    ];
    let body = proto.build_request(&req).unwrap();
    let messages = body.get("messages").unwrap().as_array().unwrap();
    assert_eq!(messages.len(), 3);

    // First two messages: plain string content (no cache_control)
    assert!(messages[0]["content"].is_string());
    assert!(messages[1]["content"].is_string());

    // Last message: array content with cache_control on last element
    let last_content = messages[2]["content"].as_array().unwrap();
    assert_eq!(last_content.len(), 1);
    assert_eq!(last_content[0]["type"], "text");
    assert_eq!(last_content[0]["text"], "Third");
    assert_eq!(
        last_content[0]["cache_control"],
        serde_json::json!({"type": "ephemeral"}),
        "cache_control must be on last message only (prefix stability)"
    );
}

/// Verify the full chain: system_blocks cache_control → AnthropicProtocol serialization.
/// Both system prompt and last message should have cache_control markers.
#[tokio::test]
async fn test_anthropic_full_chain_cache_markers_preserved() {
    use crate::protocol::ChatProtocol as _;

    let proto = AnthropicProtocol::new();
    let mut req = make_request();
    req.system_blocks = Some(vec![SystemBlock {
        text: "System prompt".to_string(),
        cache: true,
    }]);
    req.messages = vec![InternalMessage {
        role: "user".into(),
        content: "Hello".into(),
        ..Default::default()
    }];
    let body = proto.build_request(&req).unwrap();

    // System block has cache_control
    let system = body["system"].as_array().unwrap();
    assert_eq!(
        system[0]["cache_control"],
        serde_json::json!({"type": "ephemeral"}),
        "system_blocks cache marker must survive AnthropicProtocol serialization"
    );

    // Last message has cache_control
    let messages = body["messages"].as_array().unwrap();
    let last_content = messages[0]["content"].as_array().unwrap();
    assert_eq!(
        last_content[0]["cache_control"],
        serde_json::json!({"type": "ephemeral"}),
        "messages cache marker must survive AnthropicProtocol serialization"
    );
}

/// Verify that Anthropic streaming SSE events are correctly normalized
/// through the full pipeline (AnthropicProtocol parse_sse_stream).
#[tokio::test]
async fn test_anthropic_streaming_full_pipeline() {
    let client = UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(AnthropicStubProvider::new()),
        Arc::new(AnthropicProtocol::new()),
        InterpreterRegistry::new(vec![(Box::new(AnthropicInterpreter), "anthropic/*")]),
        PluginPipeline::new(),
    );
    let stream = client.chat_streaming(make_request()).await.unwrap();
    let events: Vec<_> = stream.collect().await;
    // Should have: BlockStart, BlockDelta(text), BlockEnd, MessageEnd
    assert!(
        events.len() >= 4,
        "expected at least 4 events, got {}",
        events.len()
    );
    assert!(matches!(
        events.first(),
        Some(Ok(StreamEvent::BlockStart {
            block_type: ContentBlockType::Text,
            ..
        }))
    ));
    assert!(matches!(
        events.last(),
        Some(Ok(StreamEvent::MessageEnd { .. }))
    ));
}

/// Verify error response code mapping: Anthropic error body → empty content blocks
/// (no panic, graceful degradation).
#[tokio::test]
async fn test_anthropic_error_response_mapping() {
    use crate::protocol::ChatProtocol as _;

    let proto = AnthropicProtocol::new();
    let body = serde_json::json!({
        "type": "error",
        "error": {
            "type": "authentication_error",
            "message": "Invalid API key"
        }
    });
    let resp = proto.parse_response(body).unwrap();
    // Error body has no content field → empty content blocks
    assert!(resp.content_blocks.is_empty());
    assert_eq!(resp.usage.prompt_tokens, 0);
    assert_eq!(resp.usage.completion_tokens, 0);
}

/// Verify that AnthropicInterpreter resolves correctly for anthropic/* models
/// in the assemble_llm_components registry.
#[test]
fn test_anthropic_interpreter_resolves_in_call_chain() {
    let (_, interpreter, _) = crate::call_chain::assemble_llm_components("anthropic");
    let resolved = interpreter.resolve("anthropic", "anthropic/claude-sonnet-4-20250514");
    assert_eq!(
        resolved.name(),
        "anthropic",
        "AnthropicInterpreter should be resolved for anthropic/* models"
    );
}
