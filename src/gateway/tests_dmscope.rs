//! Tests for DmScope Feishu isolation variants.

use crate::gateway::{DmScope, GatewayConfig, SessionManager};
use crate::im::{AdapterError, IMPlugin, NormalizedMessage};
use crate::llm::types::ContentBlock;
use crate::processor_chain::DslParseResult;
use crate::renderer::RenderedOutput;
use crate::session::bootstrap::BootstrapMode;
use crate::session::persistence::ReasoningLevel;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

// ── Mock plugin ─────────────────────────────────────────────────────────────

/// Mock IM plugin used by DmScope isolation tests. `platform` is configurable
/// so the same struct can be registered under different keys per test.
struct MockPlugin {
    platform: String,
    #[allow(dead_code)]
    should_fail: bool,
}

impl MockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform: platform.to_string(),
            should_fail: false,
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
}

// ── Test helpers ────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        dm_scope: DmScope::default(),
    }
}

fn make_gw(config: GatewayConfig) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
        BootstrapMode::Full,
        ReasoningLevel::default(),
    ));
    let gw = crate::gateway::Gateway::new(config, Arc::clone(&sm));
    (gw, sm)
}

/// Setup: gateway + session_manager + registered mock plugin under `channel`.
async fn setup(
    config: GatewayConfig,
    channel: &str,
) -> (crate::gateway::Gateway, Arc<SessionManager>) {
    let (gw, sm) = make_gw(config);
    gw.register_plugin(Arc::new(MockPlugin::new(channel))).await;
    (gw, sm)
}

/// Add session_id to an existing message.
async fn add_session(
    sm: &SessionManager,
    channel: &str,
    msg: &mut crate::gateway::Message,
    account_id: Option<&str>,
) {
    let sid = sm.find_or_create(channel, msg, account_id).await.unwrap();
    msg.metadata.insert("session_id".into(), sid);
}

fn feishu_msg(from: &str, to: &str) -> crate::gateway::Message {
    crate::gateway::Message {
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

// ── DmScope Feishu isolation variants ───────────────────────────────────────

#[tokio::test]
async fn test_feishu_dm_scope_isolation_variants() {
    // V1: PerChannelPeer — different open_ids → different sessions
    {
        let (gw, sm) = setup(
            GatewayConfig {
                dm_scope: DmScope::PerChannelPeer,
                ..make_config()
            },
            "feishu",
        )
        .await;
        let mut m1 = feishu_msg("ou_u1", "ag");
        let mut m2 = feishu_msg("ou_u2", "ag");
        add_session(&sm, "feishu", &mut m1, None).await;
        add_session(&sm, "feishu", &mut m2, None).await;
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 2);
        assert_ne!(sessions[0].id, sessions[1].id);
    }
    // V2: Main — all users share one session
    {
        let (gw, sm) = setup(
            GatewayConfig {
                dm_scope: DmScope::Main,
                ..make_config()
            },
            "feishu",
        )
        .await;
        let mut m1 = feishu_msg("ou_u1", "ag");
        let mut m2 = feishu_msg("ou_u2", "ag");
        add_session(&sm, "feishu", &mut m1, None).await;
        add_session(&sm, "feishu", &mut m2, None).await;
        assert_eq!(m1.metadata["session_id"], m2.metadata["session_id"],);
        assert_eq!(m1.metadata["session_id"], "feishu:ag");
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "feishu:ag");
    }
    // V3: PerAccountChannelPeer — different tenants → different sessions
    {
        let (gw, sm) = setup(
            GatewayConfig {
                dm_scope: DmScope::PerAccountChannelPeer,
                ..make_config()
            },
            "feishu",
        )
        .await;
        let mut m1 = feishu_msg("ou_u1", "ag");
        m1.metadata.insert("account_id".into(), "ta".into());
        let mut m2 = feishu_msg("ou_u1", "ag");
        m2.metadata.insert("account_id".into(), "tb".into());
        let a1 = m1.metadata.get("account_id").cloned();
        let a2 = m2.metadata.get("account_id").cloned();
        add_session(&sm, "feishu", &mut m1, a1.as_deref()).await;
        add_session(&sm, "feishu", &mut m2, a2.as_deref()).await;
        assert!(m1.metadata["session_id"].starts_with("ta:"));
        assert!(m2.metadata["session_id"].starts_with("tb:"));
        gw.route_message("feishu", m1, None).await.unwrap();
        gw.route_message("feishu", m2, None).await.unwrap();
        let sessions = gw.get_agent_sessions("ag").await;
        assert_eq!(sessions.len(), 2);
    }
}
