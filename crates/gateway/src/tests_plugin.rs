//! Tests for Gateway plugin registry (register_plugin / get_plugin).

use crate::{Gateway, GatewayConfig, SessionManager};
use async_trait::async_trait;
use closeclaw_common::im_plugin::RenderedOutput;
use closeclaw_common::im_plugin::{AdapterError, IMPlugin, NormalizedMessage};
use closeclaw_common::processor::DslParseResult;
use closeclaw_llm::types::ContentBlock;
use closeclaw_session::persistence::ReasoningLevel;
use std::sync::Arc;

// ── Helpers ────────────────────────────────────────────────────────────────

struct MockPlugin {
    platform_name: String,
    fail: bool,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl MockPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform_name: platform.to_string(),
            fail: false,
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }
    fn failing(platform: &str) -> Self {
        Self {
            platform_name: platform.to_string(),
            fail: true,
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
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

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_gw(config: GatewayConfig) -> (Gateway, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &config,
        None,
        None,
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

// ── Shutdown progress card tests (Step 1.4) ──────────────────────────────

use crate::Session;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_session::llm_session::ConversationSession;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// Mock plugin that captures the last card JSON sent via `send()`.
struct CapturingPlugin {
    platform_name: String,
    last_card: RwLock<Option<serde_json::Value>>,
    renderer: std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer>,
}

impl CapturingPlugin {
    fn new(platform: &str) -> Self {
        Self {
            platform_name: platform.to_string(),
            last_card: RwLock::new(None),
            renderer: std::sync::Mutex::new(
                crate::im_adapter::streaming::DefaultStreamingRenderer::new(),
            ),
        }
    }

    async fn last_card(&self) -> Option<serde_json::Value> {
        self.last_card.read().await.clone()
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
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
            payload: serde_json::json!({}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        _peer_id: &str,
        _thread_id: Option<&str>,
    ) -> Result<(), AdapterError> {
        if output.msg_type == "interactive" {
            *self.last_card.write().await = Some(output.payload.clone());
        }
        Ok(())
    }

    fn handle_stream_event(
        &self,
        event: closeclaw_common::processor::StreamEvent,
    ) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("CapturingPlugin streaming renderer lock poisoned")
            .handle_event(event)
    }

    fn flush_stream(&self) -> closeclaw_common::im_plugin::StreamingOutput {
        self.streaming_renderer()
            .lock()
            .expect("CapturingPlugin streaming renderer lock poisoned")
            .flush()
    }

    fn streaming_renderer(
        &self,
    ) -> &std::sync::Mutex<crate::im_adapter::streaming::DefaultStreamingRenderer> {
        &self.renderer
    }
}

/// Register a session with a ConversationSession in the given SessionManager.
async fn register_session_with_conv(
    sm: &SessionManager,
    session_id: &str,
    agent_id: &str,
    _chat_id: &str,
) {
    sm.sessions.write().await.insert(
        session_id.to_string(),
        Session {
            id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            channel: "test".to_string(),
            created_at: chrono::Utc::now().timestamp(),
            depth: 0,
        },
    );
    let cs = Arc::new(RwLock::new(ConversationSession::new(
        session_id.to_string(),
        "test-model".to_string(),
        PathBuf::from("/tmp"),
    )));
    sm.conversation_sessions
        .write()
        .await
        .insert(session_id.to_string(), cs);
}

/// Verify progress card has correct JSON structure in graceful mode.
#[tokio::test]
async fn test_progress_card_graceful_structure() {
    let (gw, sm) = make_gw(make_config());
    register_session_with_conv(&sm, "sess-1", "agent-1", "chat-1").await;

    let plugin = Arc::new(CapturingPlugin::new("test"));
    let plugin_ref = plugin.clone();
    gw.register_plugin(plugin).await;

    gw.send_shutdown_progress_card(ShutdownMode::Graceful).await;

    let card = plugin_ref.last_card().await.expect("card should be sent");

    // Header must be blue for graceful
    let header = card.get("header").expect("card must have header");
    let template = header.get("template").expect("header must have template");
    assert_eq!(template.as_str().unwrap(), "blue");

    // Header title must mention graceful shutdown
    let title = header.get("title").expect("header must have title");
    let content = title.get("content").expect("title must have content");
    assert!(content.as_str().unwrap().contains("\u{5173}\u{95ed}"));

    // Elements must contain session list and action buttons
    let elements = card
        .get("elements")
        .expect("card must have elements")
        .as_array()
        .expect("elements must be array");
    assert!(!elements.is_empty(), "elements should not be empty");

    // Last element should be action block with buttons
    let last = elements.last().unwrap();
    assert_eq!(last.get("tag").unwrap().as_str().unwrap(), "action");
    let actions = last.get("actions").expect("action must have actions");
    assert_eq!(actions.as_array().unwrap().len(), 2);
}

/// Verify progress card has correct JSON structure in forceful mode.
#[tokio::test]
async fn test_progress_card_forceful_structure() {
    let (gw, sm) = make_gw(make_config());
    register_session_with_conv(&sm, "sess-2", "agent-2", "chat-2").await;

    let plugin = Arc::new(CapturingPlugin::new("test"));
    let plugin_ref = plugin.clone();
    gw.register_plugin(plugin).await;

    gw.send_shutdown_progress_card(ShutdownMode::Forceful).await;

    let card = plugin_ref.last_card().await.expect("card should be sent");

    // Header must be red for forceful
    let header = card.get("header").expect("card must have header");
    let template = header.get("template").expect("header must have template");
    assert_eq!(template.as_str().unwrap(), "red");

    // Elements should NOT contain action buttons in forceful mode
    let elements = card
        .get("elements")
        .expect("card must have elements")
        .as_array()
        .expect("elements must be array");
    let has_action = elements.iter().any(|e| {
        e.get("tag")
            .map(|t| t.as_str() == Some("action"))
            .unwrap_or(false)
    });
    assert!(!has_action, "forceful card should not have action buttons");
}

/// No card sent when there are no active sessions.
#[tokio::test]
async fn test_progress_card_no_sessions_no_card() {
    let (gw, _sm) = make_gw(make_config());
    // Register no sessions
    let plugin = Arc::new(CapturingPlugin::new("test"));
    let plugin_ref = plugin.clone();
    gw.register_plugin(plugin).await;

    gw.send_shutdown_progress_card(ShutdownMode::Graceful).await;

    assert!(
        plugin_ref.last_card().await.is_none(),
        "no card should be sent when there are no sessions"
    );
}

/// Card send failure does not panic (fault tolerance).
#[tokio::test]
async fn test_progress_card_send_failure_does_not_panic() {
    let (gw, sm) = make_gw(make_config());
    register_session_with_conv(&sm, "sess-3", "agent-3", "chat-3").await;

    gw.register_plugin(Arc::new(MockPlugin::failing("failing")))
        .await;

    // Should not panic even though send fails
    gw.send_shutdown_progress_card(ShutdownMode::Graceful).await;
}

/// Final card has correct JSON structure.
#[tokio::test]
async fn test_shutdown_final_card_structure() {
    let (gw, sm) = make_gw(make_config());
    register_session_with_conv(&sm, "sess-4", "agent-4", "chat-4").await;

    let plugin = Arc::new(CapturingPlugin::new("test"));
    let plugin_ref = plugin.clone();
    gw.register_plugin(plugin).await;

    let result = crate::session_manager::stop::StopResult {
        succeeded: 2,
        failed: 1,
        skipped: 3,
    };
    gw.send_shutdown_final_card(&result).await;

    let card = plugin_ref
        .last_card()
        .await
        .expect("final card should be sent");

    // Header should be green for completion
    let header = card.get("header").expect("card must have header");
    let template = header.get("template").expect("header must have template");
    assert_eq!(template.as_str().unwrap(), "green");

    // Elements should contain summary with counts
    let elements = card
        .get("elements")
        .expect("card must have elements")
        .as_array()
        .expect("elements must be array");
    assert_eq!(elements.len(), 1);
    let text = elements[0]
        .get("text")
        .and_then(|t| t.get("content"))
        .and_then(|c| c.as_str())
        .unwrap();
    assert!(text.contains("2")); // succeeded
    assert!(text.contains("1")); // failed
    assert!(text.contains("3")); // skipped
}

/// Final card not sent when no sessions.
#[tokio::test]
async fn test_shutdown_final_card_no_sessions() {
    let (gw, _sm) = make_gw(make_config());
    // Register no sessions
    let plugin = Arc::new(CapturingPlugin::new("test"));
    let plugin_ref = plugin.clone();
    gw.register_plugin(plugin).await;

    let result = crate::session_manager::stop::StopResult::default();
    gw.send_shutdown_final_card(&result).await;

    assert!(
        plugin_ref.last_card().await.is_none(),
        "no final card should be sent when there are no sessions"
    );
}
