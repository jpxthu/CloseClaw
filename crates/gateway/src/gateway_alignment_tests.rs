//! Unit tests for design doc alignment changes (Steps 1.1–1.4).
//!
//! Test dimensions:
//! 1. System notifications use simplified outbound path
//! 2. Inbound queue full rejection emits debug event
//! 3. Non-streaming middleware rejection does NOT send user notification
//!    (per design doc §出站中间件 execution contract)
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
        _reply_ref: Option<&str>,
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
        inbound_wal_dir: None,
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
    gw.send_outbound_simplified(
        "chat_1",
        "mock",
        closeclaw_session::notifications::RESTORE_NOTIFICATION_DEFAULT_TEXT,
    )
    .await
    .expect("send_outbound_simplified should succeed");

    // The rejecting middleware must NOT block the simplified path.
    assert_eq!(
        plugin.send_count(),
        1,
        "simplified path should call plugin.send() despite rejecting middleware"
    );
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(
        text,
        closeclaw_session::notifications::RESTORE_NOTIFICATION_DEFAULT_TEXT
    );
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
                span_id: None,
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
            span_id: None,
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
// 3. Non-Streaming Middleware Rejection Does NOT Send User Notification
// ═════════════════════════════════════════════════════════════════════════════

/// When `log_middleware_rejection` is called (non-streaming/batch path),
/// it should NOT send a user notification via `send_outbound_simplified`.
/// Per design doc §出站中间件 execution contract: when any middleware returns
/// rejection the message is not sent and an alert log is recorded — but no
/// user notification is sent (the user simply receives nothing).
#[tokio::test]
async fn test_batch_middleware_rejection_does_not_notify_user() {
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

    let err = closeclaw_common::MiddlewareError::rejected("test-reject", "blocked by policy");
    let result = crate::outbound_helpers::log_middleware_rejection(err, "chat_test").await;
    assert!(
        result.is_ok(),
        "log_middleware_rejection should always return Ok"
    );

    // Batch middleware rejection must NOT send any notification to the user.
    assert_eq!(
        plugin.send_count(),
        0,
        "batch middleware rejection must not send user notification"
    );
}

/// When `log_middleware_rejection` receives a `MiddlewareFailed` error
/// (not `Rejected`), it should also NOT send any user notification.
#[tokio::test]
async fn test_batch_middleware_failed_does_not_notify_user() {
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
    let result = crate::outbound_helpers::log_middleware_rejection(err, "chat_test").await;
    assert!(
        result.is_ok(),
        "log_middleware_rejection should always return Ok"
    );

    assert_eq!(
        plugin.send_count(),
        0,
        "batch middleware failure must not send user notification"
    );
}

/// Streaming pre-flight middleware rejection still sends a user notification
/// via `send_outbound_simplified` (per design doc §流式模式 data flow step 1).
/// This test verifies the streaming path is NOT affected by the batch change.
#[tokio::test]
async fn test_streaming_preflight_rejection_sends_notification() {
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

    // send_outbound_simplified is the function used by streaming pre-flight
    // rejection (outbound.rs:652). Verify it still sends.
    gw.send_outbound_simplified(
        "chat_test",
        "mock",
        "Your message was not sent due to an outbound policy restriction.",
    )
    .await
    .expect("send_outbound_simplified should succeed");

    assert_eq!(
        plugin.send_count(),
        1,
        "streaming pre-flight rejection must send user notification"
    );
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(
        text, "Your message was not sent due to an outbound policy restriction.",
        "streaming pre-flight rejection notification text mismatch"
    );
}

/// Batch send failure (`notify_batch_send_failure`) still sends the
/// "⚠️ 回复发送失败" notification via simplified path (per design doc
/// §批量出错降级). This is a different scenario from middleware rejection.
#[tokio::test]
async fn test_batch_send_failure_notifies_user() {
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

    let send_error =
        closeclaw_common::im_plugin::AdapterError::SendFailed("connection refused".into());
    crate::outbound_helpers::notify_batch_send_failure(&gw, "mock", "chat_test", send_error).await;

    assert_eq!(
        plugin.send_count(),
        1,
        "batch send failure must send user notification"
    );
    let text = plugin.last_send_text().expect("should have text");
    assert_eq!(
        text, "⚠️ 回复发送失败：消息未能送达",
        "batch send failure notification text mismatch"
    );
}
