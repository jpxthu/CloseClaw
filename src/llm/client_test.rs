//! Tests for the LLM unified chat client.
//!
//! These tests live here rather than in-client to keep `client.rs` under the
//! 500-line limit imposed by the project style guide.

use futures::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::llm::interpreter::InterpreterRegistry;
use crate::llm::plugin::PluginPipeline;
use crate::llm::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use crate::llm::provider::{Provider, SseStream};
use crate::llm::types::{
    ContentBlock, ContentBlockType, ContentDelta, InternalMessage, InternalRequest,
    InternalResponse, ProtocolId, RawContentBlock, RawSseChunk, RawUsage, StreamEvent,
    UnifiedResponse, UnifiedUsage,
};

use crate::llm::client::UnifiedChatClient;

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
    ) -> crate::llm::provider::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("hello from stub".into())],
            usage: RawUsage {
                prompt_tokens: 1,
                completion_tokens: 2,
                total_tokens: Some(3),
            },
            finish_reason: Some("stop".into()),
        })
    }

    async fn send_streaming(
        &self,
        _request: InternalRequest,
        _body: serde_json::Value,
    ) -> crate::llm::provider::Result<SseStream> {
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
    ) -> crate::llm::protocol::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    fn parse_response(
        &self,
        _body: serde_json::Value,
    ) -> crate::llm::protocol::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("from protocol".into())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
            },
            finish_reason: None,
        })
    }
    fn decorate_headers(
        &self,
        _headers: &mut reqwest::header::HeaderMap,
    ) -> crate::llm::protocol::Result<()> {
        Ok(())
    }
    fn create_sse_machine(&self) -> crate::llm::types::SseStateMachine {
        crate::llm::types::SseStateMachine::new()
    }

    async fn parse_sse_stream(
        &self,
        incoming: IncomingSseStream,
        _machine: crate::llm::types::SseStateMachine,
    ) -> OutgoingEventStream {
        Box::pin(async_stream::try_stream! {
            let mut stream = incoming;
            while let Some(_chunk) = stream.next().await {
                yield StreamEvent::BlockStart { index: 0, block_type: ContentBlockType::Text };
                yield StreamEvent::BlockDelta { index: 0, delta: ContentDelta::Text { text: "hello".into() } };
                yield StreamEvent::MessageEnd {
                    usage: Some(UnifiedUsage { prompt_tokens: 1, completion_tokens: 1, total_tokens: Some(2), reasoning_tokens: None }),
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

impl crate::llm::plugin::ModelPlugin for CountingPlugin {
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
        }],
        temperature: 0.0,
        max_tokens: Some(256),
        stream: false,
        extra_body: Default::default(),
    }
}

fn make_client(pipeline: PluginPipeline) -> UnifiedChatClient {
    UnifiedChatClient::new(
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
    impl crate::llm::interpreter::ModelInterpreter for CheckInterpreter {
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
                },
                finish_reason: None,
            }
        }
        fn interpret_stream_event(&self, e: StreamEvent) -> Option<StreamEvent> {
            Some(e)
        }
    }
    let registry = InterpreterRegistry::new(vec![(Box::new(CheckInterpreter), "stub/*")]);
    let client = UnifiedChatClient::new(
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
    impl crate::llm::plugin::ModelPlugin for CapturePlugin {
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
