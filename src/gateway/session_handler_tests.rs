use super::*;
use crate::llm::client::UnifiedChatClient;
use crate::llm::fallback::FallbackClient;
use crate::llm::protocol::{ChatProtocol, IncomingSseStream, OutgoingEventStream};
use crate::llm::provider::{Provider, ProviderError};
use crate::llm::session_state::LlmState;
use crate::llm::types::ProtocolId;
use crate::llm::types::{
    InternalRequest, InternalResponse, RawContentBlock, RawUsage, SseStateMachine,
};
use crate::llm::LLMRegistry;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::Client;

#[derive(Debug, Clone)]
struct TestProvider {
    client: Client,
    headers: HeaderMap,
}
impl TestProvider {
    fn new() -> Self {
        Self {
            client: Client::new(),
            headers: HeaderMap::new(),
        }
    }
}
#[async_trait]
impl Provider for TestProvider {
    fn id(&self) -> &str {
        "test"
    }
    fn base_url(&self) -> &str {
        "http://localhost"
    }
    fn api_key(&self) -> &str {
        ""
    }
    fn supported_protocols(&self) -> &[ProtocolId] {
        &[]
    }
    fn http_client(&self) -> &Client {
        &self.client
    }
    fn default_headers(&self) -> &HeaderMap {
        &self.headers
    }
    async fn send(
        &self,
        _req: InternalRequest,
        _body: serde_json::Value,
    ) -> std::result::Result<InternalResponse, ProviderError> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("test".into())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
    async fn send_streaming(
        &self,
        _req: InternalRequest,
        _body: serde_json::Value,
    ) -> std::result::Result<crate::llm::provider::SseStream, ProviderError> {
        let (_, rx) = tokio::sync::mpsc::channel(1);
        Ok(rx)
    }
}

#[derive(Debug, Clone)]
struct TestProtocol {
    id: ProtocolId,
}
impl TestProtocol {
    fn new() -> Self {
        Self {
            id: ProtocolId::new("test"),
        }
    }
}
#[async_trait]
impl ChatProtocol for TestProtocol {
    fn protocol_id(&self) -> &ProtocolId {
        &self.id
    }
    fn path(&self) -> &str {
        "/chat"
    }
    fn build_request(
        &self,
        _req: &InternalRequest,
    ) -> crate::llm::protocol::Result<serde_json::Value> {
        Ok(serde_json::json!({}))
    }
    fn parse_response(
        &self,
        _body: serde_json::Value,
    ) -> crate::llm::protocol::Result<InternalResponse> {
        Ok(InternalResponse {
            content_blocks: vec![RawContentBlock::Text("test".into())],
            usage: RawUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            },
            finish_reason: None,
        })
    }
    fn decorate_headers(&self, _h: &mut HeaderMap) -> crate::llm::protocol::Result<()> {
        Ok(())
    }
    fn create_sse_machine(&self) -> SseStateMachine {
        SseStateMachine::new()
    }
    async fn parse_sse_stream(
        &self,
        _incoming: IncomingSseStream,
        _machine: SseStateMachine,
    ) -> OutgoingEventStream {
        Box::pin(futures::stream::empty())
    }
}

fn handler_with_sm(sm: Arc<SessionManager>) -> SessionMessageHandler {
    let registry = Arc::new(LLMRegistry::new());
    let fallback = Arc::new(FallbackClient::from_strings(registry, vec![]));
    let uc = Arc::new(UnifiedChatClient::with_noop_cache_adapter(
        Arc::new(TestProvider::new()),
        Arc::new(TestProtocol::new()),
        Default::default(),
        Default::default(),
    ));
    SessionMessageHandler::new_no_output(sm, fallback, uc)
}

fn make_msg() -> crate::gateway::Message {
    use std::collections::HashMap;
    crate::gateway::Message {
        id: "msg_1".into(),
        from: "alice".into(),
        to: "bob".into(),
        content: "hello".into(),
        channel: "ch".into(),
        timestamp: chrono::Utc::now().timestamp(),
        metadata: HashMap::new(),
        thread_id: None,
    }
}

#[tokio::test]
async fn test_idle_message_returns_llm_started() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
}

#[tokio::test]
async fn test_busy_message_returns_queued() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();

    // Manually set busy
    if let Some(cs) = sm.get_conversation_session(&sid).await {
        cs.write().await.set_llm_busy(true);
        cs.write().await.set_llm_state(LlmState::Requesting);
    }

    let handler = handler_with_sm(Arc::clone(&sm));
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued));

    // Verify message was actually enqueued
    if let Some(pending) = sm.pop_pending_message(&sid).await {
        assert_eq!(pending.content, "hello");
    } else {
        panic!("expected pending message");
    }
}

#[tokio::test]
async fn test_no_pending_no_recursion() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));

    // With empty fallback chain, call will fail — but we just verify it doesn't panic
    handler.handle_message(&sid, "hello".to_string()).await;
    // Give the task a moment to run
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // No pending messages exist
    assert!(sm.pop_pending_message(&sid).await.is_none());
}

/// After an LLM call completes (even with empty chain → failure),
/// busy should be cleared so the session becomes idle again.
#[tokio::test]
async fn test_llm_failure_resets_busy() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));

    // Start a call — busy becomes true
    let result = handler.handle_message(&sid, "hello".to_string()).await;
    assert!(matches!(result, HandleResult::LlmStarted));
    assert!(
        sm.is_session_busy(&sid).await,
        "busy should be true immediately after call"
    );

    // Wait for the async LLM task to finish (it will fail because chain is empty)
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Busy should be cleared after LLM failure
    assert!(
        !sm.is_session_busy(&sid).await,
        "busy should be reset to false after LLM failure"
    );
}

/// After an LLM call completes, pending messages are automatically drained
/// and the session handles them in order.
#[tokio::test]
async fn test_pending_consumed_after_llm_done() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));

    // First message starts LLM call, busy = true
    handler.handle_message(&sid, "first".to_string()).await;
    assert!(sm.is_session_busy(&sid).await);

    // Second message while busy → enqueued
    let result = handler.handle_message(&sid, "second".to_string()).await;
    assert!(matches!(result, HandleResult::MessageQueued));

    // Wait for first LLM call to finish and drain pending
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // After drain: no more pending (the "second" message was consumed by drain loop)
    assert!(
        sm.pop_pending_message(&sid).await.is_none(),
        "pending message should have been consumed during drain"
    );
}

/// Multiple pending messages are consumed in FIFO order.
#[tokio::test]
async fn test_multiple_pending_fifo_order() {
    let config = crate::gateway::GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let sid = sm.find_or_create("ch", &make_msg(), None).await.unwrap();
    let handler = handler_with_sm(Arc::clone(&sm));

    // Start first LLM call
    handler.handle_message(&sid, "first".to_string()).await;

    // Enqueue two more while busy
    handler.handle_message(&sid, "second".to_string()).await;
    handler.handle_message(&sid, "third".to_string()).await;

    // Verify order by draining all pending and checking order
    let mut pending = Vec::new();
    while let Some(msg) = sm.pop_pending_message(&sid).await {
        pending.push(msg);
    }
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].content, "second");
    assert_eq!(pending[1].content, "third");

    // Wait for all LLM calls to finish (first + two drained)
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // All pending should have been drained
    assert!(sm.pop_pending_message(&sid).await.is_none());
}

// `/compact` tests removed — /compact is now handled by the SlashDispatcher
// at the Gateway level, not by SessionMessageHandler. See slash_permission tests.

// `/clear` tests removed — /clear is now handled by the SlashDispatcher
// at the Gateway level, not by SessionMessageHandler. See slash_permission tests.
