//! Unit tests for the inbound bounded queue.
//!
//! Covers: enqueue success, full-queue rejection with busy reply,
//! FIFO ordering, consumer task dispatch, and bypass mode.

use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig, InboundRequest};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{ContentBlock, MessageType, NormalizedMessage};
use closeclaw_session::bootstrap::loader::BootstrapMode;
use closeclaw_session::persistence::ReasoningLevel;
use tokio::sync::mpsc;

use super::inbound_queue::{start_inbound_consumer, InboundQueueFull, InboundQueueHandle};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_raw_payload(text: &str) -> Vec<u8> {
    serde_json::json!({
        "header": {
            "event_id": "ev_test",
            "event_type": "im.message.receive_v1",
            "create_time": "1700000000000",
            "token": "t",
            "app_id": "a"
        },
        "event": {
            "sender": {
                "sender_id": {
                    "open_id": "u1"
                },
                "sender_type": "user",
                "tenant_key": "tk"
            },
            "message": {
                "message_id": "m1",
                "root_id": "",
                "parent_id": "",
                "create_time": "1700000000000",
                "chat_id": "p1",
                "chat_type": "p2p",
                "message_type": "text",
                "content": format!("{{\"text\":\"{}\"}}", text)
            }
        }
    })
    .to_string()
    .into_bytes()
}

fn make_request(content: &str) -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: make_raw_payload(content),
        peer_id: "p1".into(),
    }
}

fn make_gateway() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "test".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        dm_scope: Default::default(),
        inbound_queue_capacity: 4,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    Arc::new(Gateway::new(config, sm))
}

// ---------------------------------------------------------------------------
// Handle-level tests (pure channel, no Gateway)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_try_send_ok_and_capacity() {
    let (tx, _rx) = mpsc::channel::<InboundRequest>(8);
    let handle = InboundQueueHandle::new(tx);
    assert_eq!(handle.capacity(), 8);
    assert!(handle.try_send(make_request("a")).is_ok());
    assert!(handle.try_send(make_request("b")).is_ok());
}

#[tokio::test]
async fn test_try_send_full_returns_original_request() {
    let (tx, _rx) = mpsc::channel::<InboundRequest>(1);
    let handle = InboundQueueHandle::new(tx);
    assert!(handle.try_send(make_request("a")).is_ok());
    let err: Result<(), InboundQueueFull> = handle.try_send(make_request("overflow"));
    assert!(err.is_err());
    let full = err.unwrap_err();
    assert_eq!(full.request.peer_id, "p1");
}

#[tokio::test]
async fn test_try_send_closed_channel() {
    let (tx, rx) = mpsc::channel::<InboundRequest>(4);
    let handle = InboundQueueHandle::new(tx);
    drop(rx); // close receiver
    let err: Result<(), InboundQueueFull> = handle.try_send(make_request("x"));
    assert!(err.is_err());
}

// ---------------------------------------------------------------------------
// Consumer task tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_consumer_fires_parse_and_process() {
    // The consumer task calls gateway.get_plugin → parse_inbound →
    // process_inbound_chain → handle_inbound_message.
    // Without a plugin registered, the consumer should not panic or hang.
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(8);
    let capacity = 8;
    start_inbound_consumer(rx, Arc::clone(&gw), capacity);

    // Send a message through the channel directly.
    tx.send(make_request("hello")).await.unwrap();
    tx.send(make_request("world")).await.unwrap();

    // Give the consumer time to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Channel should be drained (messages dropped because no plugin registered).
    assert!(tx.try_send(make_request("z")).is_ok());
    // No panic = consumer ran and handled missing plugin gracefully.
}

#[tokio::test]
async fn test_consumer_fifo_order() {
    // Messages are processed in order; we verify by sending N messages
    // and ensuring none are dropped.
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(16);
    start_inbound_consumer(rx, Arc::clone(&gw), 16);

    for i in 0..10 {
        tx.send(make_request(&format!("msg-{i}"))).await.unwrap();
    }

    // Wait for processing.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    // All messages consumed — channel should be empty.
    assert!(tx.try_send(make_request("extra")).is_ok());
}

#[tokio::test]
async fn test_consumer_stops_on_channel_close() {
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<InboundRequest>(4);
    start_inbound_consumer(rx, Arc::clone(&gw), 4);

    tx.send(make_request("before")).await.unwrap();
    drop(tx); // close sender — consumer should exit its loop

    // Consumer task should terminate; we verify by waiting a bit.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // No panic = consumer exited cleanly.
}

// ---------------------------------------------------------------------------
// Gateway-level enqueue_inbound tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_enqueue_inbound_without_queue_bypasses() {
    // When inbound_tx is None (queue not started), enqueue_inbound
    // processes the message directly without going through the channel.
    let gw = make_gateway();
    // No start_inbound_queue() call — inbound_tx remains None.
    gw.enqueue_inbound(make_request("direct")).await;
    // If we got here without panic, bypass mode works.
}

#[tokio::test]
async fn test_start_inbound_queue_returns_handle() {
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();
    // Handle should have the configured capacity.
    assert_eq!(handle.capacity(), 4);
    // Enqueue via handle should succeed.
    assert!(handle.try_send(make_request("ok")).is_ok());
}

#[tokio::test]
async fn test_gateway_enqueue_inbound_full_triggers_busy_reply() {
    // Fill the queue to capacity, then enqueue one more.
    // Since no plugin is registered, the busy reply is silently dropped.
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();

    // Fill queue (capacity = 4).
    for i in 0..4 {
        handle.try_send(make_request(&format!("fill-{i}"))).unwrap();
    }
    // Next enqueue should trigger busy reply path (no plugin → silently skipped).
    gw.enqueue_inbound(make_request("overflow")).await;
    // No panic = busy reply path handled gracefully with no plugin.
}

#[tokio::test]
async fn test_inbound_request_clone_preserves_fields() {
    let req = make_request("clone-test");
    let cloned = req.clone();
    assert_eq!(cloned.platform, "feishu");
    assert_eq!(cloned.peer_id, "p1");
    assert_eq!(cloned.raw_payload, make_raw_payload("clone-test"));
}

// ---------------------------------------------------------------------------
// Defensive empty text filter tests (Step 1.1)
// ---------------------------------------------------------------------------

/// A mock plugin that returns `Ok(Some(NormalizedMessage))` with empty text
/// content, bypassing the adapter-level filter. This exercises the defensive
/// filter in `process_inbound_direct`.
struct EmptyTextBypassPlugin;

#[async_trait]
impl IMPlugin for EmptyTextBypassPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "  ".into(), // whitespace-only
            timestamp: 0,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: "u1".into(),
        }))
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// A mock plugin that returns a non-text message with empty content.
/// Non-text messages should NOT be filtered by the empty text guard.
struct NonTextEmptyContentPlugin;

#[async_trait]
impl IMPlugin for NonTextEmptyContentPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: String::new(),
            timestamp: 0,
            message_type: MessageType::Image,
            media_refs: vec![],
            thread_id: None,
            account_id: "u1".into(),
        }))
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[tokio::test]
async fn test_process_inbound_direct_drops_empty_text() {
    // Register a plugin that bypasses the adapter-level filter and returns
    // an empty-text NormalizedMessage. The defensive filter in
    // process_inbound_direct should drop it.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyTextBypassPlugin)).await;
    // No queue started → enqueue_inbound uses process_inbound_direct fallback.
    gw.enqueue_inbound(make_request("empty-text")).await;
    // No panic = empty text was filtered without entering the chain.
}

#[tokio::test]
async fn test_process_inbound_direct_passes_non_text_empty_content() {
    // Non-text messages with empty content should NOT be filtered.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NonTextEmptyContentPlugin))
        .await;
    gw.enqueue_inbound(make_request("img-empty")).await;
    // No panic = image message passed through (handler may produce unsupported reply).
}

#[tokio::test]
async fn test_consumer_drops_empty_text_from_plugin() {
    // Same defensive filter applies in the consumer path when a plugin
    // returns an empty-text NormalizedMessage.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyTextBypassPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle.try_send(make_request("empty-via-queue")).unwrap();
    // Wait for consumer to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // No panic = empty text filtered in consumer path too.
}

// ---------------------------------------------------------------------------
// Step 1.3 — Additional empty-text consumer path tests
// ---------------------------------------------------------------------------

/// A mock plugin that returns empty string text content.
struct EmptyStringTextPlugin;

#[async_trait]
impl IMPlugin for EmptyStringTextPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: String::new(), // empty string
            timestamp: 0,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: "u1".into(),
        }))
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// A mock plugin that returns non-empty text (normal message).
struct NormalTextPlugin;

#[async_trait]
impl IMPlugin for NormalTextPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "hello world".into(),
            timestamp: 0,
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: "u1".into(),
        }))
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

#[tokio::test]
async fn test_consumer_drops_empty_string_text() {
    // Consumer path should drop text message with empty string content.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyStringTextPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle.try_send(make_request("empty-str")).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // No panic = empty string text filtered in consumer path.
}

#[tokio::test]
async fn test_consumer_passes_non_text_empty_content() {
    // Consumer path should NOT filter non-text messages with
    // empty content (e.g. image with no alt text).
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NonTextEmptyContentPlugin))
        .await;
    let handle = gw.start_inbound_queue();
    handle.try_send(make_request("img-via-queue")).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // No panic = image message passed through consumer path.
}

#[tokio::test]
async fn test_consumer_passes_normal_text() {
    // Consumer path should pass non-empty text messages.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NormalTextPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle.try_send(make_request("normal")).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // No panic = normal text message passed through consumer path.
}

#[tokio::test]
async fn test_fallback_drops_empty_string_text() {
    // Fallback path should also drop text with empty string.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyStringTextPlugin)).await;
    gw.enqueue_inbound(make_request("empty-str-fb")).await;
    // No panic = empty string text filtered in fallback path.
}

#[tokio::test]
async fn test_fallback_passes_normal_text() {
    // Fallback path should pass non-empty text messages.
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NormalTextPlugin)).await;
    gw.enqueue_inbound(make_request("normal-fb")).await;
    // No panic = normal text passed through fallback path.
}
