//! Tests for the standalone `compute_session_key` function.

use crate::{compute_session_key, GatewayConfig, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage};
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock plugin ─────────────────────────────────────────────────────────────

struct MockPlugin {
    platform: String,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl MockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }
}

#[async_trait]
impl IMPlugin for MockPlugin {
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
        _content_blocks: &[ContentBlock],
        _dsl_result: Option<&DslParseResult>,
    ) -> RenderedOutput {
        RenderedOutput {
            msg_type: "text".into(),
            payload: json!({"content": {"text": ""}}),
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

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("MockPlugin streaming renderer lock poisoned")
            .handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("MockPlugin streaming renderer lock poisoned")
            .flush()
    }

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        &self.renderer
    }
}

// ── Test helpers ────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_gw(config: GatewayConfig) -> (crate::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ));
    let gw = crate::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

async fn setup(config: GatewayConfig, channel: &str) -> (crate::Gateway, Arc<SessionManager>) {
    let (gw, sm) = make_gw(config);
    gw.register_plugin(Arc::new(MockPlugin::new(channel))).await;
    (gw, sm)
}

async fn add_session(
    sm: &SessionManager,
    channel: &str,
    msg: &mut crate::Message,
    account_id: Option<&str>,
) {
    let sid = sm.find_or_create(channel, msg, account_id).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
}

fn feishu_msg(from: &str, to: &str) -> crate::Message {
    crate::Message {
        id: "x".into(),
        from: from.into(),
        to: to.into(),
        content: "hi".into(),
        channel: "feishu".into(),
        timestamp: 0,
        metadata: HashMap::new(),
        thread_id: None,
    }
}

// ── compute_session_key unit tests ─────────────────────────────────────────

#[test]
fn test_compute_session_key_format() {
    let key = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 1_700_000_000_000);
    let parts: Vec<&str> = key.splitn(2, '-').collect();
    assert_eq!(parts.len(), 2, "key must have exactly one '-' separator");
    assert_eq!(parts[0], "1700000000000", "timestamp prefix mismatch");
    assert_eq!(parts[1].len(), 64, "hash must be 64 hex chars");
    assert!(
        parts[1].chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be valid hex"
    );
}

#[test]
fn test_compute_session_key_deterministic() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 100);
    let key2 = compute_session_key("feishu", "ou_u1", "ag", Some("acc1"), 100);
    assert_eq!(key1, key2, "same inputs must produce identical key");
}

#[test]
fn test_compute_session_key_different_senders() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", None, 100);
    let key2 = compute_session_key("feishu", "ou_u2", "ag", None, 100);
    assert_ne!(key1, key2, "different senders must produce different keys");
}

#[test]
fn test_compute_session_key_different_accounts() {
    let key1 = compute_session_key("feishu", "ou_u1", "ag", Some("ta"), 100);
    let key2 = compute_session_key("feishu", "ou_u1", "ag", Some("tb"), 100);
    assert_ne!(
        key1, key2,
        "different account_ids must produce different keys"
    );
}

#[test]
fn test_compute_session_key_none_vs_default_account() {
    let key_none = compute_session_key("feishu", "ou_u1", "ag", None, 100);
    let key_default = compute_session_key("feishu", "ou_u1", "ag", Some("default"), 100);
    assert_eq!(
        key_none, key_default,
        "None and Some('default') should produce the same key"
    );
}

#[test]
fn test_compute_session_key_account_none_uses_default_string() {
    // Verify that account_id=None uses literal "default" in hash input
    let key_with_none = compute_session_key("ch", "a", "b", None, 0);
    let key_with_explicit = compute_session_key("ch", "a", "b", Some("default"), 0);
    assert_eq!(key_with_none, key_with_explicit);
}

#[test]
fn test_compute_session_key_different_channels() {
    let key1 = compute_session_key("feishu", "a", "b", None, 100);
    let key2 = compute_session_key("discord", "a", "b", None, 100);
    assert_ne!(key1, key2, "different channels must produce different keys");
}

#[test]
fn test_compute_session_key_timestamp_sensitivity() {
    let k1 = compute_session_key("feishu", "a", "b", None, 100);
    let k2 = compute_session_key("feishu", "a", "b", None, 200);
    assert_ne!(k1, k2, "different timestamps must produce different keys");
}
