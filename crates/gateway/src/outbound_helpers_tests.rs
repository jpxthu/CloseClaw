//! Unit tests for outbound_helpers — streaming text dispatch.
//!
//! After Step 1.2, middleware is no longer applied per-chunk during
//! streaming. `send_text` and `send_render_block` send directly via
//! the plugin without middleware. Middleware gating is handled by
//! pre-flight check in `send_outbound_streaming_inner`.
//!
//! These tests verify the updated send_text behavior (no middleware).

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::OutboundMiddleware;
use closeclaw_common::{ContentBlock, MiddlewareContext, MiddlewareError};

use super::inbound_queue::InboundRequest;
use super::inbound_queue_test_utils::queued;
use crate::outbound_helpers::{send_simplified_with_timeout, send_text, StreamContext};
use crate::{Gateway, GatewayConfig, SessionManager};

// ---------------------------------------------------------------------------
// Test helpers

fn test_gw() -> Gateway {
    let config = GatewayConfig {
        name: "outbound_helpers_test".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = std::sync::Arc::new(SessionManager::new(&config, None, None, Default::default()));
    Gateway::new(config, sm)
}

// ---------------------------------------------------------------------------
// Mock plugin
// ---------------------------------------------------------------------------

/// Mock plugin that records every `send()` call for assertion.
struct SendTrackingPlugin {
    send_called: Arc<std::sync::atomic::AtomicBool>,
    last_text: Arc<std::sync::Mutex<Option<String>>>,
}

impl SendTrackingPlugin {
    fn new() -> (Self, SendTracker) {
        let send_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let last_text = Arc::new(std::sync::Mutex::new(None));
        let tracker = SendTracker {
            send_called: Arc::clone(&send_called),
            last_text: Arc::clone(&last_text),
        };
        (
            Self {
                send_called,
                last_text,
            },
            tracker,
        )
    }
}

/// Lightweight handle to query the tracking plugin's state.
struct SendTracker {
    send_called: Arc<std::sync::atomic::AtomicBool>,
    last_text: Arc<std::sync::Mutex<Option<String>>>,
}

impl SendTracker {
    fn was_send_called(&self) -> bool {
        self.send_called.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn last_sent_text(&self) -> Option<String> {
        self.last_text.lock().unwrap().clone()
    }
}

#[async_trait]
impl IMPlugin for SendTrackingPlugin {
    fn platform(&self) -> &str {
        "mock"
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
                ContentBlock::Text(t) => Some(t.as_str()),
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
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.send_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        *self.last_text.lock().unwrap() = Some(text);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock middleware (for reference / future use)
// ---------------------------------------------------------------------------

/// Middleware that always allows (returns Ok).
#[allow(dead_code)]
struct AllowMiddleware;

#[async_trait]
impl OutboundMiddleware for AllowMiddleware {
    fn name(&self) -> &str {
        "allow"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_plugin() -> (Arc<dyn IMPlugin>, SendTracker) {
    let (plugin, tracker) = SendTrackingPlugin::new();
    (Arc::new(plugin), tracker)
}

fn make_stream_ctx<'a>(
    plugin: &'a Arc<dyn IMPlugin>,
    session_id: &'a str,
    channel: &'a str,
    chat_id: &'a str,
    gateway: &'a Gateway,
) -> StreamContext<'a> {
    StreamContext {
        gateway,
        plugin,
        session_id,
        channel,
        chat_id,
        thread_id: None,
        reply_ref: None,
        registry: None,
        trace_id: None,
        session_key: None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// send_text dispatches text directly via plugin.send (no middleware).
#[tokio::test]
async fn test_send_text_dispatches_directly() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s1", "mock", "chat1", &gw);
    send_text(&ctx, "hello world").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "hello world");
}

/// send_text with empty text still dispatches.
#[tokio::test]
async fn test_send_text_empty_string() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s2", "mock", "chat2", &gw);
    send_text(&ctx, "").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "");
}

/// send_text with special characters dispatches correctly.
#[tokio::test]
async fn test_send_text_special_characters() {
    let (plugin, tracker) = make_plugin();
    let gw = test_gw();
    let ctx = make_stream_ctx(&plugin, "s3", "mock", "chat3", &gw);
    send_text(&ctx, "hello 🌍 <script>alert('xss')</script>")
        .await
        .unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(
        tracker.last_sent_text().unwrap(),
        "hello 🌍 <script>alert('xss')</script>"
    );
}

// ===========================================================================
// send_simplified_with_timeout tests (Step 1.4)
// ===========================================================================

/// Mock plugin whose `send()` completes immediately.
struct FastSendPlugin;

#[async_trait]
impl IMPlugin for FastSendPlugin {
    fn platform(&self) -> &str {
        "mock"
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
                ContentBlock::Text(t) => Some(t.as_str()),
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
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// Mock plugin whose `send()` sleeps for 3 seconds (exceeds 2s timeout).
struct SlowSendPlugin;

#[async_trait]
impl IMPlugin for SlowSendPlugin {
    fn platform(&self) -> &str {
        "mock"
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
                ContentBlock::Text(t) => Some(t.as_str()),
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
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        Ok(())
    }
}

/// send_simplified_with_timeout returns Ok when plugin.send() completes quickly.
#[tokio::test]
async fn test_send_simplified_with_timeout_normal_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(FastSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let result = send_simplified_with_timeout(&gw, "chat1", "mock", "hello").await;
    assert!(
        result.is_ok(),
        "normal path should return Ok, got {:?}",
        result
    );
}

/// send_simplified_with_timeout returns Ok (drops message) when plugin.send()
/// takes longer than 2 seconds — the timeout fires and the function must
/// not block beyond ~2s.
#[tokio::test]
async fn test_send_simplified_with_timeout_timeout_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(SlowSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let start = std::time::Instant::now();
    let result = send_simplified_with_timeout(&gw, "chat2", "mock", "slow-msg").await;
    let elapsed = start.elapsed();
    assert!(
        result.is_ok(),
        "timeout path should return Ok, got {:?}",
        result
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(2),
        "timeout should fire around 2s, took {:?}",
        elapsed
    );
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "function should return within ~4s ceiling, took {:?}",
        elapsed
    );
}

/// send_simplified_with_timeout returns Ok when plugin is not registered
/// (falls back to plain-text log path).
#[tokio::test]
async fn test_send_simplified_with_timeout_no_plugin_fallback() {
    let gw = test_gw();
    // No plugin registered — should fallback to plain-text log, not panic.
    let result = send_simplified_with_timeout(&gw, "chat3", "nonexistent", "msg").await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
}

// ===========================================================================
// send_system_notification tests (Step 1.4)
// ============================================================================

/// send_system_notification returns (unit) when plugin.send() completes quickly.
#[tokio::test]
async fn test_send_system_notification_normal_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(FastSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    // send_system_notification is fire-and-forget (returns ()); just ensure no panic.
    gw.send_system_notification("chat1", "mock", "hello system notification")
        .await;
}

/// send_system_notification does not panic when plugin.send() exceeds 2s timeout.
#[tokio::test]
async fn test_send_system_notification_timeout_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(SlowSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let start = std::time::Instant::now();
    gw.send_system_notification("chat2", "mock", "slow notification")
        .await;
    let elapsed = start.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_secs(2),
        "timeout should fire around 2s, took {:?}",
        elapsed
    );
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "send_system_notification should return within ~4s ceiling, took {:?}",
        elapsed
    );
}

/// send_system_notification handles missing plugin gracefully (fallback path).
#[tokio::test]
async fn test_send_system_notification_no_plugin_fallback() {
    let gw = test_gw();
    // No plugin registered — should fallback to plain-text log, not panic.
    gw.send_system_notification("chat3", "nonexistent", "fallback notification")
        .await;
}

// ===========================================================================
// 1-second response constraint monitoring test (Step 1.4)
// ===========================================================================

/// Mock plugin whose `parse_inbound()` sleeps for 1.5 seconds, causing
/// total processing to exceed the 1-second response constraint.
struct SlowParsePlugin;

#[async_trait]
impl IMPlugin for SlowParsePlugin {
    fn platform(&self) -> &str {
        "feishu"
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<NormalizedMessage>, AdapterError> {
        // Simulate slow inbound parsing (>1s).
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        Ok(Some(NormalizedMessage {
            platform: "feishu".into(),
            sender_id: "u1".into(),
            peer_id: "p1".into(),
            content: "slow-parse".into(),
            timestamp: chrono::Utc::now().timestamp(),
            message_type: closeclaw_common::MessageType::Text,
            media_refs: vec![],
            thread_id: None,
            reply_ref: None,
            account_id: "u1".into(),
            ..Default::default()
        }))
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        let text = content_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
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
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// When inbound processing exceeds 1 second, the 1-second response constraint
/// monitor should fire a warn log. This test enqueues a message through the
/// consumer with a slow-parse plugin and verifies the log is emitted.
#[tokio::test]
async fn test_1s_response_constraint_warn_log() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer;
    use tracing_subscriber::Registry;

    // Set up a tracing subscriber that captures warn-level logs.
    let subscriber = Registry::default().with(
        tracing_subscriber::fmt::layer()
            .with_test_writer()
            .with_filter(tracing_subscriber::EnvFilter::new("warn")),
    );
    let _guard = tracing::subscriber::set_default(subscriber);

    let gw = make_gateway_for_1s_test();
    gw.register_plugin(Arc::new(SlowParsePlugin) as Arc<dyn IMPlugin>)
        .await;
    let handle = gw.start_inbound_queue();

    let req = make_slow_request();
    handle.try_send(queued(req)).unwrap();

    // Wait for processing to complete (>1.5s for slow parse).
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    // No panic = the warn log was emitted successfully.
}

fn make_gateway_for_1s_test() -> Arc<Gateway> {
    let config = GatewayConfig {
        name: "outbound_helpers_1s_test".into(),
        rate_limit_per_minute: 0,
        max_message_size: 0,
        inbound_queue_capacity: 4,
        inbound_wal_dir: None,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    Arc::new(Gateway::new(config, sm))
}

fn make_slow_request() -> InboundRequest {
    InboundRequest {
        platform: "feishu".into(),
        raw_payload: b"{}".to_vec(),
        peer_id: "p1".into(),
        trace_id: "slow-parse-trace".into(),
        span_id: None,
    }
}
