//! Unit tests for plain-text fallback in outbound message routing.
//!
//! Covers the two fallback paths added in Step 1.2:
//! - No target plugin registered → `fallback_to_plain_text` (log only)
//! - Plugin exists but render/send fails → `send_as_plain_text` (retry as plain text)

use crate::{Gateway, GatewayConfig, GatewayError, Session, SessionManager};
use closeclaw_common::im_plugin::{AdapterError, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, DslParseResult};
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock plugins
// ---------------------------------------------------------------------------

/// Mock plugin that always succeeds on send.
struct SuccessMock {
    platform: String,
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for SuccessMock {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
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
        Ok(())
    }
}

/// Mock plugin whose `send` always fails.
struct FailingSendMock {
    platform: String,
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for FailingSendMock {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        // Normal render succeeds.
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
        Err(AdapterError::SendFailed("send failed".into()))
    }
}

/// Mock plugin whose `send` always fails — used for double-failure fallback tests.
struct AlwaysFailMock {
    platform: String,
}

#[async_trait::async_trait]
impl closeclaw_common::IMPlugin for AlwaysFailMock {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(
        &self,
        _payload: &[u8],
    ) -> Result<
        Option<closeclaw_common::im_plugin::NormalizedMessage>,
        closeclaw_common::im_plugin::AdapterError,
    > {
        Ok(None)
    }

    fn render(
        &self,
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": ""}}),
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

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn test_config() -> GatewayConfig {
    GatewayConfig {
        name: "test-fallback".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

async fn make_gw(
    session_id: &str,
    channel: &str,
    plugin: Option<Arc<dyn closeclaw_common::IMPlugin>>,
) -> Gateway {
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
    if let Some(p) = plugin {
        gw.register_plugin(p).await;
    }
    gw
}

// ===========================================================================
// 1. Normal path — no target plugin: three methods return Ok(())
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_no_plugin_returns_ok() {
    let gw = make_gw("s1", "mock", None).await;
    let result = gw.send_outbound("s1", "mock", "hello", vec![]).await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_send_outbound_to_chat_no_plugin_returns_ok() {
    let gw = make_gw("s2", "mock", None).await;
    let result = gw.send_outbound_to_chat("chat_1", "mock", "hello").await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_send_outbound_simplified_no_plugin_returns_ok() {
    let gw = make_gw("s3", "mock", None).await;
    let result = gw.send_outbound_simplified("chat_1", "mock", "hello").await;
    assert!(
        result.is_ok(),
        "no-plugin fallback should return Ok, got {:?}",
        result
    );
}

// ===========================================================================
// 2. Normal path — plugin exists but send fails: plain text fallback attempted
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_send_fails_fallback_plain_text() {
    let gw = make_gw(
        "s4",
        "mock",
        Some(Arc::new(FailingSendMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound("s4", "mock", "hello", vec![]).await;
    // send fails, send_as_plain_text also fails (FailingSendMock), so error propagates.
    assert!(result.is_err(), "send failure should propagate error");
}

#[tokio::test]
async fn test_send_outbound_to_chat_send_fails_fallback() {
    let gw = make_gw(
        "s5",
        "mock",
        Some(Arc::new(FailingSendMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound_to_chat("chat_1", "mock", "hello").await;
    assert!(result.is_err(), "send failure should propagate error");
}

#[tokio::test]
async fn test_send_outbound_simplified_send_fails_fallback() {
    let gw = make_gw(
        "s6",
        "mock",
        Some(Arc::new(FailingSendMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound_simplified("chat_1", "mock", "hello").await;
    assert!(result.is_err(), "send failure should propagate error");
}

// ===========================================================================
// 3. Fallback also fails → error returned
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_double_failure_returns_error() {
    let gw = make_gw(
        "s7",
        "mock",
        Some(Arc::new(AlwaysFailMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound("s7", "mock", "hello", vec![]).await;
    assert!(result.is_err(), "double failure should return error");
}

#[tokio::test]
async fn test_send_outbound_to_chat_double_failure_returns_error() {
    let gw = make_gw(
        "s8",
        "mock",
        Some(Arc::new(AlwaysFailMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound_to_chat("chat_1", "mock", "hello").await;
    assert!(result.is_err(), "double failure should return error");
}

#[tokio::test]
async fn test_send_outbound_simplified_double_failure_returns_error() {
    let gw = make_gw(
        "s9",
        "mock",
        Some(Arc::new(AlwaysFailMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound_simplified("chat_1", "mock", "hello").await;
    assert!(result.is_err(), "double failure should return error");
}

// ===========================================================================
// 4. Log verification — warn log triggered (tracing integration)
// ===========================================================================
// We verify the fallback path is exercised by checking that the no-plugin
// scenario does not error. The warn! log is a tracing side-effect; we cannot
// assert on it directly without a subscriber guard. The test is structural:
// if the code path did NOT hit `fallback_to_plain_text`, it would return an
// error or panic instead.

#[tokio::test]
async fn test_no_plugin_exercises_fallback_path() {
    let gw = make_gw("s10", "mock", None).await;
    // send_outbound requires session_id → chat_id resolution.
    let result = gw
        .send_outbound("s10", "mock", "fallback test", vec![])
        .await;
    assert!(
        result.is_ok(),
        "fallback path should complete without error"
    );
}

#[tokio::test]
async fn test_no_plugin_to_chat_exercises_fallback_path() {
    let gw = make_gw("s11", "mock", None).await;
    let result = gw
        .send_outbound_to_chat("chat_1", "mock", "fallback test")
        .await;
    assert!(
        result.is_ok(),
        "fallback path should complete without error"
    );
}

// ===========================================================================
// 5. Control test — plugin exists and works normally
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_plugin_works_normally() {
    let gw = make_gw(
        "s12",
        "mock",
        Some(Arc::new(SuccessMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw.send_outbound("s12", "mock", "hello world", vec![]).await;
    assert!(
        result.is_ok(),
        "normal path should succeed, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_send_outbound_to_chat_plugin_works_normally() {
    let gw = make_gw(
        "s13",
        "mock",
        Some(Arc::new(SuccessMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw
        .send_outbound_to_chat("chat_1", "mock", "hello world")
        .await;
    assert!(
        result.is_ok(),
        "normal path should succeed, got {:?}",
        result
    );
}

#[tokio::test]
async fn test_send_outbound_simplified_plugin_works_normally() {
    let gw = make_gw(
        "s14",
        "mock",
        Some(Arc::new(SuccessMock {
            platform: "mock".into(),
        })),
    )
    .await;
    let result = gw
        .send_outbound_simplified("chat_1", "mock", "hello world")
        .await;
    assert!(
        result.is_ok(),
        "normal path should succeed, got {:?}",
        result
    );
}

// ===========================================================================
// 6. Boundary values
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_no_plugin_empty_channel() {
    let gw = make_gw("s15", "", None).await;
    let result = gw.send_outbound("s15", "", "hello", vec![]).await;
    assert!(result.is_ok(), "empty channel no-plugin should return Ok");
}

#[tokio::test]
async fn test_send_outbound_no_plugin_empty_raw_output() {
    let gw = make_gw("s16", "mock", None).await;
    let result = gw.send_outbound("s16", "mock", "", vec![]).await;
    assert!(
        result.is_ok(),
        "empty raw_output no-plugin should return Ok"
    );
}

#[tokio::test]
async fn test_send_outbound_to_chat_empty_channel() {
    let gw = make_gw("s17", "", None).await;
    let result = gw.send_outbound_to_chat("chat_1", "", "hello").await;
    assert!(result.is_ok(), "empty channel no-plugin should return Ok");
}

#[tokio::test]
async fn test_send_outbound_to_chat_empty_raw_output() {
    let gw = make_gw("s18", "mock", None).await;
    let result = gw.send_outbound_to_chat("chat_1", "mock", "").await;
    assert!(
        result.is_ok(),
        "empty raw_output no-plugin should return Ok"
    );
}

#[tokio::test]
async fn test_send_outbound_simplified_empty_channel() {
    let gw = make_gw("s19", "", None).await;
    let result = gw.send_outbound_simplified("chat_1", "", "hello").await;
    assert!(result.is_ok(), "empty channel no-plugin should return Ok");
}

#[tokio::test]
async fn test_send_outbound_simplified_empty_raw_output() {
    let gw = make_gw("s20", "mock", None).await;
    let result = gw.send_outbound_simplified("chat_1", "mock", "").await;
    assert!(
        result.is_ok(),
        "empty raw_output no-plugin should return Ok"
    );
}

// ===========================================================================
// 7. send_outbound — MissingSessionId when session_id is invalid
// ===========================================================================

#[tokio::test]
async fn test_send_outbound_missing_session_returns_error() {
    let gw = make_gw("s_valid", "mock", None).await;
    let result = gw
        .send_outbound("nonexistent_session", "mock", "hello", vec![])
        .await;
    assert!(
        matches!(result, Err(GatewayError::MissingSessionId)),
        "invalid session should return MissingSessionId, got {:?}",
        result
    );
}
