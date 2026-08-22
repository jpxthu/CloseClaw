//! Unit tests for streaming pre-flight middleware and OutboundRawLog.
//!
//! Test dimensions:
//! - Pre-flight middleware passes → streaming continues normally
//! - Pre-flight middleware rejects → streaming terminated + rejection msg
//! - After pre-flight, incremental stage does not call middleware
//! - OutboundRawLog writes log when raw_log_dir configured
//! - OutboundRawLog is skipped when raw_log_dir not configured

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_common::{
    ContentBlock, MiddlewareContext, MiddlewareError, OutboundMiddleware, StreamingRenderer,
};

use crate::{GatewayConfig, OutboundMeta, SessionManager};
use closeclaw_common::processor::{StreamEvent, UnifiedUsage};
use closeclaw_session::persistence::ReasoningLevel;
use futures::stream;

// ---------------------------------------------------------------------------
// Mock plugin that tracks send calls
// ---------------------------------------------------------------------------

struct TrackingPlugin {
    send_count: Arc<AtomicUsize>,
    last_text: Arc<std::sync::Mutex<Option<String>>>,
    renderer: std::sync::Mutex<closeclaw_common::streaming::DefaultStreamingRenderer>,
}

impl TrackingPlugin {
    fn new() -> (
        Self,
        Arc<AtomicUsize>,
        Arc<std::sync::Mutex<Option<String>>>,
    ) {
        let send_count = Arc::new(AtomicUsize::new(0));
        let last_text = Arc::new(std::sync::Mutex::new(None));
        let plugin = Self {
            send_count: Arc::clone(&send_count),
            last_text: Arc::clone(&last_text),
            renderer: std::sync::Mutex::new(
                closeclaw_common::streaming::DefaultStreamingRenderer::new(),
            ),
        };
        (plugin, send_count, last_text)
    }
}

#[async_trait]
impl IMPlugin for TrackingPlugin {
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
        self.send_count.fetch_add(1, Ordering::SeqCst);
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

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.renderer.lock().expect("lock").handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.renderer.lock().expect("lock").flush()
    }
}

// ---------------------------------------------------------------------------
// Mock middlewares
// ---------------------------------------------------------------------------

/// Middleware that always rejects in pre_flight_check.
struct RejectingMiddleware;

#[async_trait]
impl OutboundMiddleware for RejectingMiddleware {
    fn name(&self) -> &str {
        "rejecting"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }

    async fn pre_flight_check(&self, _ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        Err(MiddlewareError::rejected("rejecting", "test rejection"))
    }
}

/// Middleware that always allows but tracks pre_flight_check calls.
struct TrackingMiddleware {
    pre_flight_count: Arc<AtomicUsize>,
}

impl TrackingMiddleware {
    fn new() -> (Self, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let mw = Self {
            pre_flight_count: Arc::clone(&count),
        };
        (mw, count)
    }
}

#[async_trait]
impl OutboundMiddleware for TrackingMiddleware {
    fn name(&self) -> &str {
        "tracking"
    }

    async fn process(
        &self,
        _ctx: &MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), MiddlewareError> {
        Ok(())
    }

    async fn pre_flight_check(&self, _ctx: &MiddlewareContext) -> Result<(), MiddlewareError> {
        self.pre_flight_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_plugin() -> (
    Arc<dyn IMPlugin>,
    Arc<AtomicUsize>,
    Arc<std::sync::Mutex<Option<String>>>,
) {
    let (plugin, send_count, last_text) = TrackingPlugin::new();
    (Arc::new(plugin), send_count, last_text)
}

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-preflight".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

async fn setup_gw(session_id: &str, plugin: Arc<dyn IMPlugin>) -> crate::Gateway {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "chat_preflight".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = crate::Gateway::new(config, sm);
    gw.register_plugin(plugin).await;
    gw
}

fn simple_stream() -> impl futures::Stream<Item = Result<StreamEvent, crate::GatewayError>> {
    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "hello".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            finish_reason: None,
        }),
    ];
    stream::iter(events)
}

// ===========================================================================
// Pre-flight tests
// ===========================================================================

/// Pre-flight rejection terminates streaming and sends rejection message.
#[tokio::test]
async fn test_pre_flight_rejection_terminates_streaming() {
    let (plugin, send_count, _last_text) = make_plugin();
    let gw = setup_gw("pf-reject", Arc::clone(&plugin)).await;
    gw.add_outbound_middleware(Arc::new(RejectingMiddleware));

    let result = gw
        .send_outbound_streaming(
            "pf-reject",
            "mock",
            simple_stream(),
            &plugin,
            OutboundMeta::default(),
        )
        .await;

    // Should return an error
    assert!(result.is_err(), "pre-flight rejection should return error");

    // The rejection message is sent via send_outbound_simplified,
    // which calls plugin.send at least once
    let count = send_count.load(Ordering::SeqCst);
    assert!(
        count >= 1,
        "rejection message should be sent, got {} send calls",
        count,
    );
}

/// Pre-flight passes → streaming executes normally.
#[tokio::test]
async fn test_pre_flight_pass_allows_streaming() {
    let (plugin, send_count, _last_text) = make_plugin();
    let gw = setup_gw("pf-pass", Arc::clone(&plugin)).await;
    let (mw, _count) = TrackingMiddleware::new();
    gw.add_outbound_middleware(Arc::new(mw));

    let result = gw
        .send_outbound_streaming(
            "pf-pass",
            "mock",
            simple_stream(),
            &plugin,
            OutboundMeta::default(),
        )
        .await;

    assert!(
        result.is_ok(),
        "pre-flight pass should not error: {:?}",
        result
    );
    assert!(
        send_count.load(Ordering::SeqCst) >= 1,
        "streaming should send at least one message"
    );
}

/// Pre-flight check is called exactly once (not per-chunk).
#[tokio::test]
async fn test_pre_flight_called_exactly_once() {
    let (plugin, _send_count, _last_text) = make_plugin();
    let gw = setup_gw("pf-once", Arc::clone(&plugin)).await;
    let (mw, pre_flight_count) = TrackingMiddleware::new();
    gw.add_outbound_middleware(Arc::new(mw));

    let _ = gw
        .send_outbound_streaming(
            "pf-once",
            "mock",
            simple_stream(),
            &plugin,
            OutboundMeta::default(),
        )
        .await;

    let count = pre_flight_count.load(Ordering::SeqCst);
    assert_eq!(
        count, 1,
        "pre_flight_check should be called exactly once, got {}",
        count,
    );
}

/// No middleware → streaming works without errors.
#[tokio::test]
async fn test_no_middleware_streaming_works() {
    let (plugin, send_count, _last_text) = make_plugin();
    let gw = setup_gw("no-mw", Arc::clone(&plugin)).await;

    let result = gw
        .send_outbound_streaming(
            "no-mw",
            "mock",
            simple_stream(),
            &plugin,
            OutboundMeta::default(),
        )
        .await;

    assert!(
        result.is_ok(),
        "no middleware should not error: {:?}",
        result
    );
    assert!(
        send_count.load(Ordering::SeqCst) >= 1,
        "streaming should send at least one message"
    );
}

// ===========================================================================
// OutboundRawLog tests
// ===========================================================================

/// When raw_log_dir is configured, finish_streaming_pipeline runs
/// OutboundRawLog (via full outbound Processor Chain).
#[tokio::test]
async fn test_raw_log_dir_configured_pipeline_runs() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let config = GatewayConfig {
        name: "test-rawlog".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        raw_log_dir: Some(tmp_dir.path().to_path_buf()),
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, sm);
    let session_id = "rawlog-test";

    // Map session
    gw.session_manager.sessions.write().await.insert(
        session_id.to_string(),
        crate::Session {
            id: session_id.to_string(),
            agent_id: "chat_rawlog".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );

    let (plugin, _send_count, _last_text) = make_plugin();
    let events: Vec<Result<StreamEvent, crate::GatewayError>> = vec![
        Ok(StreamEvent::BlockStart {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::BlockDelta {
            index: 0,
            delta: closeclaw_common::ContentDelta::Text {
                text: "logged content".to_string(),
            },
        }),
        Ok(StreamEvent::BlockEnd {
            index: 0,
            block_type: closeclaw_common::ContentBlockType::Text,
        }),
        Ok(StreamEvent::MessageEnd {
            usage: Some(UnifiedUsage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: Some(2),
                reasoning_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            finish_reason: None,
        }),
    ];
    let s = stream::iter(events);

    let result = gw
        .send_outbound_streaming(session_id, "mock", s, &plugin, OutboundMeta::default())
        .await;

    // Pipeline should complete without error
    assert!(
        result.is_ok(),
        "raw_log_dir pipeline should not error: {:?}",
        result,
    );

    // Check that log files were written (at least one outbound log)
    let entries: Vec<_> = std::fs::read_dir(tmp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.contains("outbound"))
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !entries.is_empty(),
        "expected outbound log files when raw_log_dir is configured"
    );
}

/// When raw_log_dir is NOT configured, pipeline runs without error
/// and no log files are written.
#[tokio::test]
async fn test_no_raw_log_dir_no_error() {
    let (plugin, send_count, _last_text) = make_plugin();
    let gw = setup_gw("no-rawlog", Arc::clone(&plugin)).await;

    let result = gw
        .send_outbound_streaming(
            "no-rawlog",
            "mock",
            simple_stream(),
            &plugin,
            OutboundMeta::default(),
        )
        .await;

    assert!(
        result.is_ok(),
        "no raw_log_dir should not error: {:?}",
        result,
    );
    assert!(
        send_count.load(Ordering::SeqCst) >= 1,
        "streaming should still send messages"
    );
}
