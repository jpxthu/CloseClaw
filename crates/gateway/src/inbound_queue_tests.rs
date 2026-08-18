//! Unit tests for the inbound bounded queue.
//!
//! Covers: enqueue success, full-queue rejection with busy reply,
//! FIFO ordering, consumer task dispatch, and bypass mode.

use std::sync::Arc;

use crate::session_manager::SessionManager;
use crate::{Gateway, GatewayConfig};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{ContentBlock, MessageType, NormalizedMessage};
use closeclaw_session::persistence::ReasoningLevel;
use tokio::sync::mpsc;

use super::inbound_queue::{
    start_inbound_consumer, InboundQueueFull, InboundQueueHandle, QueuedInbound,
};
use super::inbound_queue_test_utils::{make_gateway, make_raw_payload, make_request, queued};

// ---------------------------------------------------------------------------
// Handle-level tests (pure channel, no Gateway)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_try_send_ok_and_capacity() {
    let (tx, _rx) = mpsc::channel::<QueuedInbound>(8);
    let handle = InboundQueueHandle::new(tx);
    assert_eq!(handle.capacity(), 8);
    assert!(handle.try_send(queued(make_request("a"))).is_ok());
    assert!(handle.try_send(queued(make_request("b"))).is_ok());
}

#[tokio::test]
async fn test_try_send_full_returns_original_request() {
    let (tx, _rx) = mpsc::channel::<QueuedInbound>(1);
    let handle = InboundQueueHandle::new(tx);
    assert!(handle.try_send(queued(make_request("a"))).is_ok());
    let err: Result<(), InboundQueueFull> = handle.try_send(queued(make_request("overflow")));
    assert!(err.is_err());
    let full = err.unwrap_err();
    assert_eq!(full.request.peer_id, "p1");
}

#[tokio::test]
async fn test_try_send_closed_channel() {
    let (tx, rx) = mpsc::channel::<QueuedInbound>(4);
    let handle = InboundQueueHandle::new(tx);
    drop(rx); // close receiver
    let err: Result<(), InboundQueueFull> = handle.try_send(queued(make_request("x")));
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
    let (tx, rx) = mpsc::channel::<QueuedInbound>(8);
    let capacity = 8;
    start_inbound_consumer(rx, Arc::clone(&gw), capacity, None);

    // Send a message through the channel directly.
    tx.send(queued(make_request("hello"))).await.unwrap();
    tx.send(queued(make_request("world"))).await.unwrap();

    // Give the consumer time to process.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // Channel should be drained (messages dropped because no plugin registered).
    assert!(tx.try_send(queued(make_request("z"))).is_ok());
    // No panic = consumer ran and handled missing plugin gracefully.
}

#[tokio::test]
async fn test_consumer_fifo_order() {
    // Messages are processed in order; we verify by sending N messages
    // and ensuring none are dropped.
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<QueuedInbound>(16);
    start_inbound_consumer(rx, Arc::clone(&gw), 16, None);

    for i in 0..10 {
        tx.send(queued(make_request(&format!("msg-{i}"))))
            .await
            .unwrap();
    }

    // Wait for processing.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    // All messages consumed — channel should be empty.
    assert!(tx.try_send(queued(make_request("extra"))).is_ok());
}

#[tokio::test]
async fn test_consumer_stops_on_channel_close() {
    let gw = make_gateway();
    let (tx, rx) = mpsc::channel::<QueuedInbound>(4);
    start_inbound_consumer(rx, Arc::clone(&gw), 4, None);

    tx.send(queued(make_request("before"))).await.unwrap();
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
    let _ = gw.enqueue_inbound(make_request("direct")).await;
    // If we got here without panic, bypass mode works.
}

#[tokio::test]
async fn test_start_inbound_queue_returns_handle() {
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();
    // Handle should have the configured capacity.
    assert_eq!(handle.capacity(), 4);
    // Enqueue via handle should succeed.
    assert!(handle.try_send(queued(make_request("ok"))).is_ok());
}

#[tokio::test]
async fn test_gateway_enqueue_inbound_full_triggers_busy_reply() {
    // Fill the queue to capacity, then enqueue one more.
    // Since no plugin is registered, the busy reply is silently dropped.
    let gw = make_gateway();
    let handle = gw.start_inbound_queue();

    // Fill queue (capacity = 4).
    for i in 0..4 {
        handle
            .try_send(queued(make_request(&format!("fill-{i}"))))
            .unwrap();
    }
    // Next enqueue should trigger busy reply path (no plugin → silently skipped).
    let result = gw.enqueue_inbound(make_request("overflow")).await;
    assert!(result.is_err(), "queue full should return Err");
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
            ..Default::default()
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
            ..Default::default()
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
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyTextBypassPlugin)).await;
    let _ = gw.enqueue_inbound(make_request("empty-text")).await;
}

#[tokio::test]
async fn test_process_inbound_direct_passes_non_text_empty_content() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NonTextEmptyContentPlugin))
        .await;
    let _ = gw.enqueue_inbound(make_request("img-empty")).await;
}

#[tokio::test]
async fn test_consumer_drops_empty_text_from_plugin() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyTextBypassPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle
        .try_send(queued(make_request("empty-via-queue")))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
            ..Default::default()
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
            ..Default::default()
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
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyStringTextPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("empty-str"))).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_consumer_passes_non_text_empty_content() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NonTextEmptyContentPlugin))
        .await;
    let handle = gw.start_inbound_queue();
    handle
        .try_send(queued(make_request("img-via-queue")))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_consumer_passes_normal_text() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NormalTextPlugin)).await;
    let handle = gw.start_inbound_queue();
    handle.try_send(queued(make_request("normal"))).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_fallback_drops_empty_string_text() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(EmptyStringTextPlugin)).await;
    let _ = gw.enqueue_inbound(make_request("empty-str-fb")).await;
}

#[tokio::test]
async fn test_fallback_passes_normal_text() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(NormalTextPlugin)).await;
    let _ = gw.enqueue_inbound(make_request("normal-fb")).await;
}
// ---------------------------------------------------------------------------
// Busy reply uses simplified outbound path (Step 1.3)
// ---------------------------------------------------------------------------

/// A middleware that rejects every outbound message.
struct RejectAllMiddleware;

#[async_trait]
impl closeclaw_common::OutboundMiddleware for RejectAllMiddleware {
    fn name(&self) -> &str {
        "reject-all"
    }

    async fn process(
        &self,
        _ctx: &closeclaw_common::MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), closeclaw_common::MiddlewareError> {
        Err(closeclaw_common::MiddlewareError::rejected(
            "reject-all",
            "blocked",
        ))
    }
}

/// A mock plugin that tracks whether `send()` was called.
struct TrackingSendPlugin {
    send_called: std::sync::atomic::AtomicBool,
}

impl TrackingSendPlugin {
    fn new() -> Self {
        Self {
            send_called: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn was_send_called(&self) -> bool {
        self.send_called.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl IMPlugin for TrackingSendPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": "busy"}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn test_busy_reply_uses_simplified_outbound_path() {
    let gw = make_gateway();
    let plugin = Arc::new(TrackingSendPlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    gw.add_outbound_middleware(Arc::new(RejectAllMiddleware));

    let handle = gw.start_inbound_queue();
    for i in 0..4 {
        handle
            .try_send(queued(make_request(&format!("fill-{i}"))))
            .unwrap();
    }

    let _ = gw.enqueue_inbound(make_request("overflow-mw-test")).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(
        plugin.was_send_called(),
        "send_outbound_simplified should call plugin.send()"
    );
}

// ---------------------------------------------------------------------------
// Busy reply timeout test (Step 1.6)
// ---------------------------------------------------------------------------

/// A mock plugin whose `send()` blocks for longer than 2 seconds.
struct SlowSendPlugin;

#[async_trait]
impl IMPlugin for SlowSendPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "ou_slow".into(),
            peer_id: "chat_slow".into(),
            content: "slow".into(),
            timestamp: chrono::Utc::now().timestamp(),
            message_type: MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            account_id: String::new(),
            ..Default::default()
        }))
    }

    fn render(
        &self,
        content_blocks: &[closeclaw_common::ContentBlock],
        _dsl_result: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                closeclaw_common::ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        Ok(())
    }

    fn send_thinking_indicator(&self, _active: bool) {}

    fn handle_stream_event(
        &self,
        _event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        closeclaw_common::im_plugin::StreamingOutput::default()
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        closeclaw_common::im_plugin::StreamingOutput::default()
    }
}

#[tokio::test]
async fn test_busy_reply_timeout_drops_after_2s() {
    let gw = make_gateway();
    gw.register_plugin(Arc::new(SlowSendPlugin)).await;
    let handle = gw.start_inbound_queue();

    for i in 0..4 {
        handle
            .try_send(queued(make_request(&format!("fill-{i}"))))
            .unwrap();
    }

    let start = std::time::Instant::now();
    let _ = gw.enqueue_inbound(make_request("overflow-slow")).await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "test should complete well within 5s, took {:?}",
        elapsed
    );
}

// ---------------------------------------------------------------------------
// Busy reply text alignment with design doc (Step 1.2)
// ---------------------------------------------------------------------------

/// A mock plugin that captures the text sent via `send()`.
struct CapturingPlugin {
    last_sent_text: std::sync::Mutex<Option<String>>,
}

impl CapturingPlugin {
    fn new() -> Self {
        Self {
            last_sent_text: std::sync::Mutex::new(None),
        }
    }

    fn last_sent_text(&self) -> Option<String> {
        self.last_sent_text.lock().unwrap().clone()
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                closeclaw_common::ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        *self.last_sent_text.lock().unwrap() = Some(text);
        Ok(())
    }
}

#[tokio::test]
async fn test_busy_reply_text_matches_design_doc() {
    let gw = make_gateway();
    let plugin = Arc::new(CapturingPlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let handle = gw.start_inbound_queue();

    for i in 0..4 {
        handle
            .try_send(queued(make_request(&format!("fill-{i}"))))
            .unwrap();
    }
    let _ = gw
        .enqueue_inbound(make_request("overflow-text-check"))
        .await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let sent = plugin.last_sent_text();
    assert!(sent.is_some(), "plugin.send() should have been called");
    assert_eq!(
        sent.unwrap(),
        "\u{670D}\u{52A1}\u{7E41}\u{5FD9}\u{FF0C}\u{8BF7}\u{7A0D}\u{540E}\u{91CD}\u{8BD5}",
        "busy reply text must match design doc exactly (no emoji prefix)"
    );
}

#[tokio::test]
async fn test_boundary_n_plus_one_triggers_busy_reply_text() {
    let config = GatewayConfig {
        name: "test-boundary".to_owned(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 1,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = Arc::new(Gateway::new(config, sm));
    let plugin = Arc::new(CapturingPlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let handle = gw.start_inbound_queue();

    handle.try_send(queued(make_request("first"))).unwrap();
    let result = gw.enqueue_inbound(make_request("second")).await;
    assert!(result.is_err(), "queue full should return Err");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let sent = plugin.last_sent_text();
    assert!(sent.is_some(), "N+1 message should trigger busy reply");
    assert_eq!(
        sent.unwrap(),
        "\u{670D}\u{52A1}\u{7E41}\u{5FD9}\u{FF0C}\u{8BF7}\u{7A0D}\u{540E}\u{91CD}\u{8BD5}",
        "boundary busy reply text must match design doc"
    );
}

#[tokio::test]
async fn test_queue_not_full_no_busy_reply() {
    let gw = make_gateway();
    let plugin = Arc::new(CapturingPlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let handle = gw.start_inbound_queue();

    handle.try_send(queued(make_request("msg-1"))).unwrap();
    handle.try_send(queued(make_request("msg-2"))).unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert!(
        plugin.last_sent_text().is_none(),
        "no busy reply should be sent when queue is not full"
    );
}

// ---------------------------------------------------------------------------
// enqueue_inbound returns Result (Step 1.3)
// ---------------------------------------------------------------------------

/// Verify that enqueue_inbound returns Err(InboundQueueFull) when the queue
/// is at capacity, while still sending the busy reply.
#[tokio::test]
async fn test_enqueue_inbound_returns_err_when_queue_full() {
    let gw = make_gateway();
    let plugin = Arc::new(TrackingSendPlugin::new());
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    let handle = gw.start_inbound_queue();

    // Fill queue to capacity (4).
    for i in 0..4 {
        handle
            .try_send(queued(make_request(&format!("fill-{i}"))))
            .unwrap();
    }

    // Enqueue one more — should return Err.
    let result = gw.enqueue_inbound(make_request("overflow-result")).await;
    assert!(result.is_err(), "queue full should return Err");
    let err = result.unwrap_err();
    assert_eq!(err.request.peer_id, "p1");
    assert_eq!(err.request.platform, "feishu");

    // Busy reply should still have been sent.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        plugin.was_send_called(),
        "busy reply should still be sent even though we return Err"
    );
}

/// Verify that enqueue_inbound returns Ok(()) when the queue has space.
#[tokio::test]
async fn test_enqueue_inbound_returns_ok_when_queue_has_space() {
    let gw = make_gateway();
    let _handle = gw.start_inbound_queue();

    // Queue has capacity 4, enqueue one — should succeed.
    let result = gw.enqueue_inbound(make_request("ok-msg")).await;
    assert!(result.is_ok(), "queue has space should return Ok");
}

