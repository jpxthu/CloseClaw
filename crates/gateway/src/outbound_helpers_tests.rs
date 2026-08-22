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

use crate::outbound_helpers::{send_text, StreamContext};

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
) -> StreamContext<'a> {
    StreamContext {
        plugin,
        session_id,
        channel,
        chat_id,
        thread_id: None,
        registry: None,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// send_text dispatches text directly via plugin.send (no middleware).
#[tokio::test]
async fn test_send_text_dispatches_directly() {
    let (plugin, tracker) = make_plugin();
    let ctx = make_stream_ctx(&plugin, "s1", "mock", "chat1");
    send_text(&ctx, "hello world").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "hello world");
}

/// send_text with empty text still dispatches.
#[tokio::test]
async fn test_send_text_empty_string() {
    let (plugin, tracker) = make_plugin();
    let ctx = make_stream_ctx(&plugin, "s2", "mock", "chat2");
    send_text(&ctx, "").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "");
}

/// send_text with special characters dispatches correctly.
#[tokio::test]
async fn test_send_text_special_characters() {
    let (plugin, tracker) = make_plugin();
    let ctx = make_stream_ctx(&plugin, "s3", "mock", "chat3");
    send_text(&ctx, "hello 🌍 <script>alert('xss')</script>")
        .await
        .unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(
        tracker.last_sent_text().unwrap(),
        "hello 🌍 <script>alert('xss')</script>"
    );
}
