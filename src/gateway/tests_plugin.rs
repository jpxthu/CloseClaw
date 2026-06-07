//! Tests for Gateway plugin registry (register_plugin / get_plugin).

use crate::gateway::{Gateway, GatewayConfig, SessionManager};
use crate::im::{AdapterError, IMPlugin, NormalizedMessage};
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use crate::renderer::RenderedOutput;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use async_trait::async_trait;
use std::sync::Arc;

// ── Helpers ────────────────────────────────────────────────────────────────

struct MockPlugin {
    platform_name: String,
    fail: bool,
}

impl MockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform_name: platform.to_string(),
            fail: false,
        }
    }
    fn failing(platform: &str) -> Self {
        Self {
            platform_name: platform.to_string(),
            fail: true,
        }
    }
}

#[async_trait]
impl IMPlugin for MockPlugin {
    fn platform(&self) -> &str {
        &self.platform_name
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
            payload: serde_json::json!({"content": {"text": "mock"}}),
        }
    }
    async fn send(
        &self,
        _output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        if self.fail {
            Err(AdapterError::SendFailed("mock failure".into()))
        } else {
            Ok(())
        }
    }
}

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: crate::gateway::DmScope::default(),
        ..Default::default()
    }
}

fn make_gw(config: GatewayConfig) -> (Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Minimal,
        ReasoningLevel::default(),
    ));
    let gw = Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

// ── Plugin registry tests ──────────────────────────────────────────────────

#[tokio::test]
async fn test_register_plugin_inserts_into_registry() {
    let (gw, _sm) = make_gw(make_config());
    let plugin = Arc::new(MockPlugin::new("alpha"));
    gw.register_plugin(plugin).await;
    let retrieved = gw.get_plugin("alpha").await;
    assert!(retrieved.is_some(), "registered plugin must be retrievable");
    assert_eq!(retrieved.unwrap().platform(), "alpha");
}

#[tokio::test]
async fn test_get_plugin_unknown_returns_none() {
    let (gw, _sm) = make_gw(make_config());
    let result = gw.get_plugin("not-registered").await;
    assert!(
        result.is_none(),
        "unknown platform must yield None from get_plugin"
    );
}

#[tokio::test]
async fn test_register_plugin_replaces_existing_entry() {
    let (gw, _sm) = make_gw(make_config());
    let p1: Arc<dyn IMPlugin> = Arc::new(MockPlugin::new("shared"));
    let p2: Arc<dyn IMPlugin> = Arc::new(MockPlugin::failing("shared"));
    gw.register_plugin(p1.clone()).await;
    gw.register_plugin(p2.clone()).await;

    // Re-registering the same platform must replace the prior Arc.
    let retrieved = gw.get_plugin("shared").await.expect("plugin must exist");
    assert!(
        Arc::ptr_eq(&retrieved, &p2),
        "second registration must replace the first"
    );
    assert!(
        !Arc::ptr_eq(&retrieved, &p1),
        "first plugin should no longer be reachable"
    );
}

#[tokio::test]
async fn test_register_plugin_uses_plugin_platform_as_key() {
    // The registry key is derived from the plugin's `platform()` method,
    // not from any caller-supplied string. Registering a plugin that reports
    // `"beta"` should NOT be reachable under any other key.
    let (gw, _sm) = make_gw(make_config());
    let plugin = Arc::new(MockPlugin::new("beta"));
    gw.register_plugin(plugin).await;
    assert!(gw.get_plugin("beta").await.is_some());
    assert!(gw.get_plugin("BETA").await.is_none());
    assert!(gw.get_plugin("").await.is_none());
}
