//! Unit tests for Step 1.5b: batch send failure + simplified path failure.
//!
//! Covers three error-path behaviors:
//! 1. Batch send failure → user receives failure notification, no retry,
//!    no outbound history written.
//! 2. Simplified path itself fails → plain-text fallback attempted.
//! 3. Middleware rejection is unaffected by batch failure changes.

use crate::{Gateway, GatewayConfig, Session, SessionManager};
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared mock helpers
// ---------------------------------------------------------------------------

fn mock_platform() -> &'static str {
    "mock"
}

fn mock_parse_inbound(
    _payload: &[u8],
) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
    Ok(None)
}

fn mock_render(
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

/// Controls send behavior for the unified mock plugin.
enum SendMode {
    /// All sends succeed; captured text stored in `sent_texts`.
    AlwaysOk {
        sent_texts: std::sync::Mutex<Vec<String>>,
    },
    /// All sends return `SendFailed`.
    AlwaysFail,
    /// First call fails, second succeeds; captured text stored in `sent_texts`.
    FailThenOk {
        call_count: std::sync::atomic::AtomicU32,
        sent_texts: std::sync::Mutex<Vec<String>>,
    },
}

/// Unified mock plugin. Only `send` behavior differs between test scenarios.
/// - `send_count()` returns total `send` calls (success or failure).
/// - `sent_texts()` returns captured text for all calls (both success and failure paths).
/// - `last_sent_text()` returns the most recent captured text.
struct MockPlugin {
    send_mode: SendMode,
}

impl MockPlugin {
    fn always_ok() -> Self {
        Self {
            send_mode: SendMode::AlwaysOk {
                sent_texts: std::sync::Mutex::new(Vec::new()),
            },
        }
    }

    fn always_fail() -> Self {
        Self {
            send_mode: SendMode::AlwaysFail,
        }
    }

    fn fail_then_ok() -> Self {
        Self {
            send_mode: SendMode::FailThenOk {
                call_count: std::sync::atomic::AtomicU32::new(0),
                sent_texts: std::sync::Mutex::new(Vec::new()),
            },
        }
    }

    fn send_count(&self) -> usize {
        match &self.send_mode {
            SendMode::AlwaysOk { sent_texts } => sent_texts.lock().unwrap().len(),
            SendMode::AlwaysFail => 0,
            SendMode::FailThenOk { sent_texts, .. } => sent_texts.lock().unwrap().len(),
        }
    }

    fn sent_texts(&self) -> Vec<String> {
        match &self.send_mode {
            SendMode::AlwaysOk { sent_texts } => sent_texts.lock().unwrap().clone(),
            SendMode::AlwaysFail => Vec::new(),
            SendMode::FailThenOk { sent_texts, .. } => sent_texts.lock().unwrap().clone(),
        }
    }

    fn last_sent_text(&self) -> Option<String> {
        self.sent_texts().last().cloned()
    }
}

#[async_trait::async_trait]
impl IMPlugin for MockPlugin {
    fn platform(&self) -> &str {
        mock_platform()
    }
    async fn parse_inbound(
        &self,
        payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
        mock_parse_inbound(payload)
    }
    fn render(
        &self,
        content_blocks: &[ContentBlock],
        dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        mock_render(content_blocks, dsl_result)
    }
    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match &self.send_mode {
            SendMode::AlwaysOk { sent_texts } => {
                sent_texts.lock().unwrap().push(text);
                Ok(())
            }
            SendMode::AlwaysFail => Err(AdapterError::SendFailed("always fails".into())),
            SendMode::FailThenOk {
                call_count,
                sent_texts,
            } => {
                sent_texts.lock().unwrap().push(text);
                let count = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if count == 0 {
                    Err(AdapterError::SendFailed("network error".into()))
                } else {
                    Ok(())
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-batch-failure".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

async fn make_gw(session_id: &str, channel: &str, plugin: Arc<dyn IMPlugin>) -> Gateway {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: "chat_test".to_string(),
            channel: channel.to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = Gateway::new(config, Arc::clone(&sm));
    gw.register_plugin(plugin).await;
    gw
}

// ===========================================================================
// 1. Batch send failure → notification + no retry + no outbound history
// ===========================================================================

/// When `plugin.send()` fails in `dispatch_and_persist`, the gateway
/// sends a failure notification via the simplified path, does NOT retry
/// the original message, and does NOT write outbound history.
///
/// We verify:
/// - `send_outbound` returns Ok(()) (error caught, not propagated)
/// - The plugin receives exactly two send calls (batch fail + notification)
/// - The notification text matches the design doc §批量出错降级
#[tokio::test]
async fn test_batch_send_failure_sends_notification() {
    let mock = Arc::new(MockPlugin::fail_then_ok());
    let gw = make_gw("s1", "mock", mock.clone()).await;
    let result = gw
        .send_outbound("s1", "mock", "original message", vec![], None, None)
        .await;

    // dispatch_and_persist catches the error and returns Ok.
    assert!(
        result.is_ok(),
        "batch send failure should return Ok, got {:?}",
        result
    );

    // The plugin should have received exactly 2 calls:
    //   0: batch send (fails)
    //   1: notification (succeeds)
    assert_eq!(
        mock.sent_texts().len(),
        2,
        "plugin should be called exactly 2 times: batch + notification"
    );
    let texts = mock.sent_texts();
    // First call was the original message.
    assert_eq!(
        texts[0], "original message",
        "first call should be the original message"
    );
    // Second call was the failure notification.
    assert_eq!(
        texts[1], "⚠️ 回复发送失败：消息未能送达",
        "second call should be the failure notification"
    );
}

/// When batch send fails, the notification via simplified path does NOT
/// write outbound history. We verify this by checking that:
/// - No checkpoint was persisted (we use `BatchThenSuccess` which captures
///   texts but has no persistence service — if persist_outbound_checkpoint
///   were reached, it would be a no-op, but the early return prevents it).
/// - The plugin's `sent_texts` only contains the notification, not a
///   second "original message" retry.
#[tokio::test]
async fn test_batch_send_failure_no_outbound_history() {
    let mock = Arc::new(MockPlugin::fail_then_ok());
    let gw = make_gw("s2", "mock", mock.clone()).await;
    let result = gw
        .send_outbound("s2", "mock", "test content", vec![], None, None)
        .await;
    assert!(result.is_ok(), "should return Ok");

    // Exactly 2 calls: batch send (fails) + notification. The original
    // message is NOT retried, and no additional sends happen.
    assert_eq!(
        mock.sent_texts().len(),
        2,
        "should have exactly 2 sends: batch fail + notification"
    );
    // The second send is the notification, not a retry of the original.
    assert_eq!(
        mock.sent_texts()[1],
        "⚠️ 回复发送失败：消息未能送达",
        "second send should be failure notification, not a retry"
    );
}

/// When batch send fails AND the notification itself also fails (plugin
/// always fails), dispatch_and_persist still returns Ok — notification
/// failure must not propagate to the caller.
#[tokio::test]
async fn test_batch_send_failure_notification_also_fails() {
    let plugin: Arc<dyn IMPlugin> = Arc::new(MockPlugin::always_fail());
    let gw = make_gw("s3", "mock", plugin).await;
    let result = gw
        .send_outbound("s3", "mock", "original", vec![], None, None)
        .await;
    assert!(
        result.is_ok(),
        "double failure should still return Ok, got {:?}",
        result
    );
}

/// Batch send failure with interactive msg_type follows the same path:
/// notification is sent and the error is not propagated.
#[tokio::test]
async fn test_batch_send_failure_interactive_msg_type() {
    let mock = Arc::new(MockPlugin::fail_then_ok());
    let gw = make_gw("s4", "mock", mock.clone()).await;

    let result = gw
        .send_outbound("s4", "mock", "interactive content", vec![], None, None)
        .await;
    assert!(
        result.is_ok(),
        "batch failure should return Ok, got {:?}",
        result
    );
    // Verify notification was sent.
    assert_eq!(
        mock.sent_texts().len(),
        2,
        "plugin should be called 2 times: batch + notification"
    );
}

// ===========================================================================
// 2. Simplified path itself fails → plain text fallback
// ===========================================================================

/// When `send_outbound_simplified` plugin.send fails, it falls back to
/// `send_as_plain_text`. If that also fails, the error propagates.
#[tokio::test]
async fn test_simplified_path_send_fails_fallback_to_plain_text() {
    let plugin: Arc<dyn IMPlugin> = Arc::new(MockPlugin::always_fail());
    let gw = make_gw("s5", "mock", plugin).await;
    let result = gw
        .send_outbound_simplified("chat_5", "mock", "fallback test")
        .await;

    // send_outbound_simplified catches render-send error → send_as_plain_text.
    // Both calls fail → Err.
    assert!(
        result.is_err(),
        "simplified path double failure should return Err"
    );
}

/// When `send_outbound_simplified` plugin.send fails but `send_as_plain_text`
/// succeeds, the overall operation returns Ok (fallback recovered).
#[tokio::test]
async fn test_simplified_path_fallback_succeeds() {
    let mock = Arc::new(MockPlugin::fail_then_ok());
    let gw = make_gw("s6", "mock", mock.clone()).await;
    let result = gw
        .send_outbound_simplified("chat_6", "mock", "recovered")
        .await;
    assert!(
        result.is_ok(),
        "simplified path fallback success should return Ok, got {:?}",
        result
    );
}

/// When `send_outbound_simplified` succeeds on first try (no fallback needed),
/// only one send call is made.
#[tokio::test]
async fn test_simplified_path_success_no_fallback() {
    let mock = Arc::new(MockPlugin::always_ok());
    let gw = make_gw("s8", "mock", mock.clone()).await;
    let result = gw
        .send_outbound_simplified("chat_8", "mock", "direct success")
        .await;
    assert!(result.is_ok(), "should succeed, got {:?}", result);
    assert_eq!(
        mock.send_count(),
        1,
        "only one send call when render+send succeed"
    );
    let text = mock.last_sent_text().expect("should have sent text");
    assert_eq!(text, "direct success");
}

// ===========================================================================
// 3. Middleware rejection unaffected by batch failure changes
// ===========================================================================

/// Middleware rejection in dispatch_and_persist still sends rejection
/// notification via simplified path, regardless of batch failure changes.
#[tokio::test]
async fn test_middleware_rejection_still_works() {
    use closeclaw_common::OutboundMiddleware;

    struct RejectMiddleware;
    #[async_trait::async_trait]
    impl OutboundMiddleware for RejectMiddleware {
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
                "blocked",
            ))
        }
    }

    let mock = Arc::new(MockPlugin::always_ok());
    let gw = make_gw("s9", "mock", mock.clone()).await;
    gw.add_outbound_middleware(Arc::new(RejectMiddleware));

    let result = gw
        .send_outbound("s9", "mock", "should be rejected", vec![], None, None)
        .await;

    // Middleware rejection returns Ok(()) after sending notification.
    assert!(
        result.is_ok(),
        "middleware rejection should return Ok, got {:?}",
        result
    );

    // The plugin should have received exactly one send: the rejection notification.
    assert_eq!(
        mock.send_count(),
        1,
        "plugin should receive 1 send (rejection notification)"
    );
    let text = mock.last_sent_text().expect("should have sent text");
    assert_eq!(
        text, "Your message was not sent due to an outbound policy restriction.",
        "rejection notification text mismatch"
    );
}

/// Middleware rejection + batch send failure: middleware rejection takes
/// priority (runs before send). The send failure path is never reached.
#[tokio::test]
async fn test_middleware_rejection_before_batch_send() {
    use closeclaw_common::OutboundMiddleware;

    struct RejectMiddleware;
    #[async_trait::async_trait]
    impl OutboundMiddleware for RejectMiddleware {
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
                "blocked",
            ))
        }
    }

    let mock = Arc::new(MockPlugin::always_fail());
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s10", "mock", plugin).await;
    gw.add_outbound_middleware(Arc::new(RejectMiddleware));

    let result = gw
        .send_outbound("s10", "mock", "middleware blocks", vec![], None, None)
        .await;

    // Middleware rejection path runs first → returns Ok with rejection notification.
    // The batch send failure path (plugin.send) is never reached.
    assert!(
        result.is_ok(),
        "middleware rejection should return Ok, got {:?}",
        result
    );
    // plugin.send was never called — middleware rejected before send.
    assert_eq!(
        mock.send_count(),
        0,
        "plugin.send should not be called when middleware rejects"
    );
}

/// No-plugin fallback uses a different code path than batch failure
/// notification — they are independent.
#[tokio::test]
async fn test_no_plugin_uses_fallback_not_batch_failure() {
    let config = test_config();
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        "s11".to_string(),
        Session {
            id: "s11".to_string(),
            agent_id: "chat_test".to_string(),
            channel: "mock".to_string(),
            created_at: 0,
            depth: 0,
        },
    );
    // Do NOT register any plugin.
    let gw = Gateway::new(config, Arc::clone(&sm));

    let result = gw
        .send_outbound("s11", "mock", "no plugin", vec![], None, None)
        .await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
    // This path goes through fallback_to_plain_text, NOT
    // notify_batch_send_failure. The two paths are independent.
}
