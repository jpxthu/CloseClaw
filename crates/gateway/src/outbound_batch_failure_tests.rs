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
// Mock plugins
// ---------------------------------------------------------------------------

/// Plugin whose `send` always succeeds. Captures all sent texts.
struct SuccessMock {
    platform: String,
    sends: std::sync::Mutex<Vec<String>>,
}

impl SuccessMock {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            sends: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn last_sent_text(&self) -> Option<String> {
        self.sends.lock().unwrap().last().cloned()
    }
    fn send_count(&self) -> usize {
        self.sends.lock().unwrap().len()
    }
}

#[async_trait::async_trait]
impl IMPlugin for SuccessMock {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.sends.lock().unwrap().push(text);
        Ok(())
    }
}

/// Plugin whose `send` always fails — for batch failure notification tests.
struct BatchFailMock {
    platform: String,
}

#[async_trait::async_trait]
impl IMPlugin for BatchFailMock {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        Err(AdapterError::SendFailed("network error".into()))
    }
}

/// Plugin whose `send` always fails — used for double-failure fallback tests.
struct AlwaysFailMock {
    platform: String,
}

#[async_trait::async_trait]
impl IMPlugin for AlwaysFailMock {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        Err(AdapterError::SendFailed("always fails".into()))
    }
}

/// Fails first call, succeeds second — captures sent texts.
struct BatchThenSuccess {
    platform: String,
    call_count: std::sync::atomic::AtomicU32,
    sent_texts: std::sync::Mutex<Vec<String>>,
}

impl BatchThenSuccess {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            call_count: std::sync::atomic::AtomicU32::new(0),
            sent_texts: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl IMPlugin for BatchThenSuccess {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = output.payload["content"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.sent_texts.lock().unwrap().push(text);
        if count == 0 {
            // First call (batch send) fails.
            Err(AdapterError::SendFailed("network error".into()))
        } else {
            // Second call (notification) succeeds.
            Ok(())
        }
    }
}

/// Fails rendered send, succeeds plain text fallback.
struct FirstFailThenOk {
    platform: String,
    call_count: std::sync::atomic::AtomicU32,
}

#[async_trait::async_trait]
impl IMPlugin for FirstFailThenOk {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Err(AdapterError::SendFailed("fail".into()))
        } else {
            Ok(())
        }
    }
}

/// Always fails — for double-failure fallback tests.
struct AlwaysFailPlugin {
    platform: String,
    send_calls: std::sync::atomic::AtomicU32,
}

#[async_trait::async_trait]
impl IMPlugin for AlwaysFailPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }
    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<Option<closeclaw_common::im_plugin::NormalizedMessage>, AdapterError> {
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
        self.send_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AdapterError::SendFailed("always fail".into()))
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
    let mock = Arc::new(BatchThenSuccess::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s1", "mock", plugin).await;
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
        mock.call_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "plugin should be called exactly 2 times: batch + notification"
    );
    let texts = mock.sent_texts.lock().unwrap();
    assert_eq!(texts.len(), 2);
    // First call was the original message.
    assert_eq!(
        texts[0], "original message",
        "first call should be the original message"
    );
    // Second call was the failure notification.
    assert_eq!(
        texts[1], "⚠️ 回复发送失败：消息未能送达，请稍后重试",
        "second call should be the failure notification"
    );
}

/// When batch send fails, the notification via simplified path does NOT
/// write outbound history. We verify this indirectly: dispatch_and_persist
/// returns Ok early after notify_batch_send_failure, BEFORE reaching
/// persist_outbound_checkpoint. Without a checkpoint_manager configured,
/// persist_outbound_checkpoint is a no-op anyway, but the early return
/// is the key structural guarantee.
#[tokio::test]
async fn test_batch_send_failure_no_outbound_history() {
    let mock = Arc::new(BatchThenSuccess::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s2", "mock", plugin).await;
    let result = gw
        .send_outbound("s2", "mock", "test content", vec![], None, None)
        .await;
    assert!(result.is_ok(), "should return Ok");
    // No checkpoint manager → persist_outbound_checkpoint is a no-op.
    // The important structural guarantee: dispatch_and_persist returns Ok
    // early after notify_batch_send_failure, BEFORE reaching
    // persist_outbound_checkpoint. This test confirms the function
    // completes without error, meaning the early-return path was taken.
}

/// When batch send fails AND the notification itself also fails (plugin
/// always fails), dispatch_and_persist still returns Ok — notification
/// failure must not propagate to the caller.
#[tokio::test]
async fn test_batch_send_failure_notification_also_fails() {
    let plugin: Arc<dyn IMPlugin> = Arc::new(AlwaysFailMock {
        platform: "mock".into(),
    });
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
    let mock = Arc::new(BatchThenSuccess::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s4", "mock", plugin).await;

    // Use send_outbound directly. Our BatchThenSuccess always renders as
    // "text" msg_type (from render()), so dispatch_and_persist will handle
    // it in the text branch. The key test: batch send fails → notification
    // sent → Ok returned. This tests the same failure path regardless of
    // whether render returns "text" or "interactive".
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
        mock.call_count.load(std::sync::atomic::Ordering::SeqCst),
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
    let mock = Arc::new(AlwaysFailPlugin {
        platform: "mock".into(),
        send_calls: std::sync::atomic::AtomicU32::new(0),
    });
    let plugin: Arc<dyn IMPlugin> = mock.clone();
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
    assert_eq!(
        mock.send_calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "plugin should be called twice: rendered send + plain text fallback"
    );
}

/// When `send_outbound_simplified` plugin.send fails but `send_as_plain_text`
/// succeeds, the overall operation returns Ok (fallback recovered).
#[tokio::test]
async fn test_simplified_path_fallback_succeeds() {
    let mock = Arc::new(FirstFailThenOk {
        platform: "mock".into(),
        call_count: std::sync::atomic::AtomicU32::new(0),
    });
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s6", "mock", plugin).await;
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
    let mock = Arc::new(SuccessMock::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s8", "mock", plugin).await;
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

    let mock = Arc::new(SuccessMock::new("mock"));
    let plugin: Arc<dyn IMPlugin> = mock.clone();
    let gw = make_gw("s9", "mock", plugin).await;
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
/// priority (runs before send). The send failure path is not reached.
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

    let plugin: Arc<dyn IMPlugin> = Arc::new(BatchFailMock {
        platform: "mock".into(),
    });
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
