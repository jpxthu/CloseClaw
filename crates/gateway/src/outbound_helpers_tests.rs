//! Unit tests for outbound_helpers — streaming text middleware chain.
//!
//! Covers the plan Step 1.3 test targets:
//! - Normal path: middleware passes → text sent via plugin.send()
//! - Middleware rejection: middleware rejects → text NOT sent, warning logged
//! - Empty middleware: no middlewares registered → text sent directly
//! - Multiple middlewares: all pass → text sent

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
/// Uses `Arc<AtomicBool>` so callers can share a cheap reference to check.
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
// Mock middlewares
// ---------------------------------------------------------------------------

/// Middleware that always allows (returns Ok).
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

/// Middleware that always rejects.
struct RejectMiddleware {
    reason: String,
}

impl RejectMiddleware {
    fn new(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
        }
    }
}

#[async_trait]
impl OutboundMiddleware for RejectMiddleware {
    fn name(&self) -> &str {
        "reject"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Err(MiddlewareError::rejected("reject", &self.reason))
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
    middlewares: &'a [Arc<dyn OutboundMiddleware>],
) -> StreamContext<'a> {
    StreamContext {
        plugin,
        session_id,
        channel,
        chat_id,
        thread_id: None,
        middlewares,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

/// Normal path: empty middlewares → text sent directly.
#[tokio::test]
async fn test_send_text_empty_middlewares_sends() {
    let (plugin, tracker) = make_plugin();
    let ctx = make_stream_ctx(&plugin, "s1", "mock", "chat1", &[]);
    send_text(&ctx, "hello world").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "hello world");
}

/// Normal path: middleware passes → text sent.
#[tokio::test]
async fn test_send_text_middleware_passes_sends() {
    let (plugin, tracker) = make_plugin();
    let mws: Vec<Arc<dyn OutboundMiddleware>> = vec![Arc::new(AllowMiddleware)];
    let ctx = make_stream_ctx(&plugin, "s2", "mock", "chat2", &mws);
    send_text(&ctx, "allowed").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "allowed");
}

/// Middleware rejection: middleware rejects → text NOT sent, Ok(()) returned.
#[tokio::test]
async fn test_send_text_middleware_rejects_skips_send() {
    let (plugin, tracker) = make_plugin();
    let mws: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(RejectMiddleware::new("rate limited"))];
    let ctx = make_stream_ctx(&plugin, "s3", "mock", "chat3", &mws);
    let result = send_text(&ctx, "should not send").await;
    assert!(result.is_ok(), "middleware rejection should return Ok(())");
    assert!(
        !tracker.was_send_called(),
        "plugin.send() should not be called when middleware rejects"
    );
}

/// Multiple middlewares: all pass → text sent.
#[tokio::test]
async fn test_send_text_multiple_middlewares_all_pass() {
    let (plugin, tracker) = make_plugin();
    let mws: Vec<Arc<dyn OutboundMiddleware>> =
        vec![Arc::new(AllowMiddleware), Arc::new(AllowMiddleware)];
    let ctx = make_stream_ctx(&plugin, "s4", "mock", "chat4", &mws);
    send_text(&ctx, "multi-pass").await.unwrap();
    assert!(tracker.was_send_called());
    assert_eq!(tracker.last_sent_text().unwrap(), "multi-pass");
}

/// Multiple middlewares: first passes, second rejects → text NOT sent.
#[tokio::test]
async fn test_send_text_second_middleware_rejects() {
    let (plugin, tracker) = make_plugin();
    let mws: Vec<Arc<dyn OutboundMiddleware>> = vec![
        Arc::new(AllowMiddleware),
        Arc::new(RejectMiddleware::new("blocked by second")),
    ];
    let ctx = make_stream_ctx(&plugin, "s5", "mock", "chat5", &mws);
    let result = send_text(&ctx, "rejected by second").await;
    assert!(result.is_ok());
    assert!(
        !tracker.was_send_called(),
        "plugin.send() should not be called when second middleware rejects"
    );
}
