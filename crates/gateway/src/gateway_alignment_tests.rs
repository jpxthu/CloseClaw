//! Unit tests for design doc alignment changes (Steps 1.1–1.4).
//!
//! Test dimensions:
//! 1. System notifications use simplified outbound path
//! 2. Inbound queue full rejection emits debug event
//! 3. Non-streaming middleware rejection sends user notification
//! 4. Post-send checkpoint persistence works, no pre-send checkpoint

use crate::inbound_queue::QueuedInbound;
use crate::{GatewayConfig, InboundRequest, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Mock plugin that captures send calls ────────────────────────────────────

struct CaptureSendPlugin {
    platform: String,
    sends: std::sync::Mutex<Vec<(RenderedOutput, String, Option<String>)>>,
}

impl CaptureSendPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sends: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn send_count(&self) -> usize {
        self.sends.lock().unwrap().len()
    }

    fn last_send_text(&self) -> Option<String> {
        self.sends.lock().unwrap().last().map(|(o, _, _)| {
            o.payload["content"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
    }
}

#[async_trait]
impl IMPlugin for CaptureSendPlugin {
    fn platform(&self) -> &str {
        &self.platform
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
        peer_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        self.sends.lock().unwrap().push((
            RenderedOutput {
                msg_type: output.msg_type.clone(),
                payload: output.payload.clone(),
            },
            peer_id.to_string(),
            thread_id.map(|s| s.to_string()),
        ));
        Ok(())
    }
}

// ── Reject middleware ────────────────────────────────────────────────────────

struct RejectMiddleware;

#[async_trait]
impl closeclaw_common::OutboundMiddleware for RejectMiddleware {
    fn name(&self) -> &str {
        "test-reject"
    }

    async fn process(
        &self,
        _ctx: &closeclaw_common::MiddlewareContext,
        _rendered: &RenderedOutput,
    ) -> Result<(), closeclaw_common::MiddlewareError> {
        Err(closeclaw_common::MiddlewareError::rejected(
            "test-reject",
            "blocked by policy",
        ))
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-alignment".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        inbound_queue_capacity: 1,
        ..Default::default()
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. System Notifications Use Simplified Outbound Path
// ═════════════════════════════════════════════════════════════════════════════

/// System notifications (workflow blocked, restore) use `send_outbound_simplified`
/// which bypasses VerbosityFilter/DslParser/outbound middleware.
///
/// Strategy: register a rejecting middleware. If system notifications used
/// the full outbound path, the middleware would reject them and the plugin
/// would never receive the send call. Since they use the simplified path,
/// the middleware is skipped and the plugin receives the notification.
#[tokio::test]
async fn test_simplified_path_skips_middleware() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let plugin = Arc::new(CaptureSendPlugin::new("mock"));
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    gw.add_outbound_middleware(Arc::new(RejectMiddleware));

    // Call send_outbound_simplified directly — this is the path system
    // notifications use after Step 1.1.
    gw.send_outbound_simplified("chat_1", "mock", "正在恢复会话...")
        .await
        .expect("send_outbound_simplified should succeed");

    // The rejecting middleware must NOT block the simplified path.
    assert_eq!(
        plugin.send_count(),
        1,
        "simplified path should call plugin.send() despite rejecting middleware"
    );
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(text, "正在恢复会话...");
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Inbound Queue Full Rejection Emits Debug Event
// ═════════════════════════════════════════════════════════════════════════════

/// When the inbound queue is full and `enqueue_inbound` rejects a message,
/// `emit_debug_event` writes a "queue.rejected" event with the trace_id,
/// platform, and peer_id to the DebugLog.
#[tokio::test]
async fn test_queue_full_rejection_emits_debug_event() {
    use closeclaw_debug_log::{DebugLog, DebugLogConfig, LogLevel};
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("TempDir::new failed");
    let debug_log = DebugLog::new(DebugLogConfig {
        min_level: LogLevel::Trace,
        log_dir: temp_dir.path().to_path_buf(),
        retention_days: 1,
        redaction_patterns: vec![],
    })
    .await
    .expect("DebugLog::new failed");

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.set_debug_log(debug_log).await;

    let gw = Arc::new(gw);
    let handle = gw.start_inbound_queue();
    // Fill the queue (capacity=1).
    handle
        .try_send(QueuedInbound {
            request: InboundRequest {
                platform: "feishu".to_string(),
                raw_payload: b"{}".to_vec(),
                peer_id: "p_fill".to_string(),
                trace_id: "trace-fill".to_string(),
            },
        })
        .expect("first send should succeed");

    // This enqueue triggers queue-full rejection + emit_debug_event.
    let result = gw
        .enqueue_inbound(InboundRequest {
            platform: "feishu".to_string(),
            raw_payload: b"{}".to_vec(),
            peer_id: "p_rejected".to_string(),
            trace_id: "trace-rejected".to_string(),
        })
        .await;
    assert!(result.is_err(), "queue full should return Err");

    // Give the spawned debug-log task time to write.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Read JSONL files in the log dir and look for queue.rejected event.
    let mut found = false;
    for entry in std::fs::read_dir(temp_dir.path()).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let content = std::fs::read_to_string(&path).expect("read jsonl");
            for line in content.lines() {
                if line.contains("queue.rejected")
                    && line.contains("trace-rejected")
                    && line.contains("feishu")
                {
                    found = true;
                    break;
                }
            }
        }
        if found {
            break;
        }
    }
    assert!(
        found,
        "queue.rejected event with trace_id should be written to DebugLog"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Non-Streaming Middleware Rejection Sends User Notification
// ═════════════════════════════════════════════════════════════════════════════

/// When `log_middleware_rejection` is called (non-streaming path), it should
/// send a user notification via `send_outbound_simplified` with the standard
/// rejection message, consistent with the streaming path.
#[tokio::test]
async fn test_middleware_rejection_sends_user_notification() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let plugin = Arc::new(CaptureSendPlugin::new("mock"));
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;
    // Register a rejecting middleware — it should NOT affect the
    // simplified path used by log_middleware_rejection.
    gw.add_outbound_middleware(Arc::new(RejectMiddleware));

    let err = closeclaw_common::MiddlewareError::rejected("test-reject", "blocked by policy");
    crate::outbound_helpers::log_middleware_rejection(&gw, err, "chat_test", "mock")
        .await
        .expect("log_middleware_rejection should not error");

    // The plugin should have received exactly one send call with the
    // standard rejection notification text.
    assert_eq!(plugin.send_count(), 1, "notification should be sent once");
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(
        text, "Your message was not sent due to an outbound policy restriction.",
        "rejection notification text mismatch"
    );
}

/// When `log_middleware_rejection` receives a `MiddlewareFailed` error
/// (not `Rejected`), it should still send the same user notification.
#[tokio::test]
async fn test_middleware_failed_sends_user_notification() {
    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    let plugin = Arc::new(CaptureSendPlugin::new("mock"));
    gw.register_plugin(Arc::clone(&plugin) as Arc<dyn IMPlugin>)
        .await;

    let err = closeclaw_common::MiddlewareError::middleware_failed("buggy-mw", "mock failure");
    crate::outbound_helpers::log_middleware_rejection(&gw, err, "chat_test", "mock")
        .await
        .expect("log_middleware_rejection should not error");

    assert_eq!(plugin.send_count(), 1, "notification should be sent once");
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(
        text, "Your message was not sent due to an outbound policy restriction.",
        "rejection notification text mismatch for MiddlewareFailed"
    );
}

/// When the simplified-path plugin send fails, log_middleware_rejection
/// still returns Ok(()) — notification failure must not propagate.
#[tokio::test]
async fn test_middleware_rejection_send_failure_no_panic() {
    use closeclaw_common::im_plugin::StreamingOutput;
    use closeclaw_common::processor::StreamEvent;

    struct FailingPlugin;

    #[async_trait]
    impl IMPlugin for FailingPlugin {
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
        ) -> Result<(), AdapterError> {
            Err(AdapterError::SendFailed("mock failure".into()))
        }
        fn handle_stream_event(&self, _event: StreamEvent) -> StreamingOutput {
            StreamingOutput::default()
        }
        fn flush_stream(&self) -> StreamingOutput {
            StreamingOutput::default()
        }
    }

    let config = make_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(Arc::new(FailingPlugin)).await;

    let err = closeclaw_common::MiddlewareError::rejected("mw", "test");
    let result =
        crate::outbound_helpers::log_middleware_rejection(&gw, err, "chat_test", "mock").await;
    assert!(
        result.is_ok(),
        "log_middleware_rejection should return Ok even when send fails"
    );
}
