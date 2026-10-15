//! Unit tests for the system notification interface's audience
//! declaration (outbound-flow design doc「定向收件人」).
//!
//! Split from `outbound_helpers_tests.rs` to keep both files within the
//! 1000-line limit (CONTRIBUTING.md). Covers the Owner-only delivery
//! gate and the legacy `All` behavior of
//! `Gateway::send_system_notification`, including the timeout and
//! fallback paths of the interface itself.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_session::llm_session::NotificationAudience;

use crate::outbound_notification_test_utils::RecordingPlugin;
use crate::{Gateway, GatewayConfig, SessionManager};

fn test_gw() -> Gateway {
    let config = GatewayConfig {
        name: "system_notification_test".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    };
    let sm = Arc::new(SessionManager::new(&config, None, None, Default::default()));
    Gateway::new_for_tests(config, sm)
}

/// Mock plugin that completes `send()` immediately (used by the timeout
/// test's counterpart fast path).
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

/// Mock plugin whose `send()` sleeps 3s — exercises the interface's 2s
/// timeout in the timeout-path test.
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

// ===========================================================================
// send_system_notification basic paths (Step 1.4)
// ============================================================================

/// send_system_notification returns (unit) when plugin.send() completes quickly.
#[tokio::test]
async fn test_send_system_notification_normal_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(FastSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    // send_system_notification is fire-and-forget (returns ()); just ensure no panic.
    gw.send_system_notification(
        "chat1",
        "mock",
        "hello system notification",
        NotificationAudience::All,
        None,
    )
    .await;
}

/// send_system_notification does not panic when plugin.send() exceeds 2s timeout.
#[tokio::test]
async fn test_send_system_notification_timeout_path() {
    let gw = test_gw();
    gw.register_plugin(Arc::new(SlowSendPlugin) as Arc<dyn IMPlugin>)
        .await;
    let start = std::time::Instant::now();
    gw.send_system_notification(
        "chat2",
        "mock",
        "slow notification",
        NotificationAudience::All,
        None,
    )
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
    gw.send_system_notification(
        "chat3",
        "nonexistent",
        "fallback notification",
        NotificationAudience::All,
        None,
    )
    .await;
}

// ===========================================================================
// Owner-targeted system notification (outbound-flow design doc「定向收件人」)
// ============================================================================

/// OwnerOnly declaration + current user is the Owner → delivered to the chat.
#[tokio::test]
async fn test_owner_only_notification_delivered_to_owner() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = test_gw();
    gw.register_plugin(Arc::new(RecordingPlugin::new(Arc::clone(&messages))) as Arc<dyn IMPlugin>)
        .await;
    gw.send_system_notification(
        "chat-owner",
        "mock",
        "[缓存命中异常] secret notice",
        NotificationAudience::OwnerOnly,
        Some("owner"),
    )
    .await;
    let delivered = messages.lock().unwrap().clone();
    assert_eq!(
        delivered.len(),
        1,
        "OwnerOnly + owner sender must be delivered, got: {delivered:?}"
    );
    assert_eq!(delivered[0], "[缓存命中异常] secret notice");
}

/// OwnerOnly declaration + current user is NOT the Owner → silently
/// dropped (no error, plugin.send never invoked).
#[tokio::test]
async fn test_owner_only_notification_dropped_for_non_owner() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = test_gw();
    gw.register_plugin(Arc::new(RecordingPlugin::new(Arc::clone(&messages))) as Arc<dyn IMPlugin>)
        .await;
    gw.send_system_notification(
        "chat-other",
        "mock",
        "[缓存命中异常] secret notice",
        NotificationAudience::OwnerOnly,
        Some("user-a"),
    )
    .await;
    let delivered = messages.lock().unwrap().clone();
    assert!(
        delivered.is_empty(),
        "OwnerOnly + non-owner sender must be dropped, got: {delivered:?}"
    );
}

/// OwnerOnly declaration + unresolvable sender (None) → silently dropped.
#[tokio::test]
async fn test_owner_only_notification_dropped_without_sender() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = test_gw();
    gw.register_plugin(Arc::new(RecordingPlugin::new(Arc::clone(&messages))) as Arc<dyn IMPlugin>)
        .await;
    gw.send_system_notification(
        "chat-anon",
        "mock",
        "[缓存命中异常] secret notice",
        NotificationAudience::OwnerOnly,
        None,
    )
    .await;
    let delivered = messages.lock().unwrap().clone();
    assert!(
        delivered.is_empty(),
        "OwnerOnly + missing sender must be dropped, got: {delivered:?}"
    );
}

/// Default `All` audience → always delivered regardless of sender role.
#[tokio::test]
async fn test_all_audience_always_delivered() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let gw = test_gw();
    gw.register_plugin(Arc::new(RecordingPlugin::new(Arc::clone(&messages))) as Arc<dyn IMPlugin>)
        .await;
    // Non-owner sender: still delivered (legacy behavior unchanged).
    gw.send_system_notification(
        "chat-all",
        "mock",
        "plain notice",
        NotificationAudience::All,
        Some("user-a"),
    )
    .await;
    // Missing sender: still delivered.
    gw.send_system_notification(
        "chat-all",
        "mock",
        "plain notice 2",
        NotificationAudience::All,
        None,
    )
    .await;
    let delivered = messages.lock().unwrap().clone();
    assert_eq!(
        delivered.len(),
        2,
        "All audience must always be delivered, got: {delivered:?}"
    );
    assert_eq!(delivered[0], "plain notice");
    assert_eq!(delivered[1], "plain notice 2");
}
