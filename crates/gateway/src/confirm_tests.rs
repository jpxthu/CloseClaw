//! Unit tests for `try_handle_plan_confirm_command` prefix parsing and
//! plan confirmation command interception (Step 1.3).
//!
//! Test dimensions:
//! 1. Normal path: `/confirm <id>` → returns Some(ApprovalProcessed), confirm called
//! 2. Normal path: `/cancel <id>` → returns Some(ApprovalProcessed), cancel called
//! 3. Boundary: message without `/confirm` or `/cancel` prefix → returns None
//! 4. Boundary: `/confirm` without id → returns None (warn logged)
//! 5. Boundary: empty string → returns None
//! 6. Non-owner sender + `/confirm` → returns Some(ApprovalProcessed) + rejection message
//! 7. Non-owner sender + `/cancel` → returns Some(ApprovalProcessed) + rejection message
//! 8. None sender + `/confirm` → returns Some(ApprovalProcessed) + rejection message
//! 9. No handler configured → returns None

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, NormalizedMessage, RenderedOutput};
use closeclaw_common::plan_confirm_handler::PlanConfirmationHandler;
use closeclaw_common::processor::ContentBlock;
use closeclaw_common::IMPlugin;

use crate::{GatewayConfig, HandleResult, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;

// ── Mock PlanConfirmationHandler ─────────────────────────────────────────

struct MockConfirmHandler {
    confirm_called: Arc<AtomicBool>,
    cancel_called: Arc<AtomicBool>,
    confirm_count: Arc<AtomicUsize>,
    cancel_count: Arc<AtomicUsize>,
    confirm_result: bool,
    cancel_result: bool,
}

impl MockConfirmHandler {
    fn new() -> Self {
        Self {
            confirm_called: Arc::new(AtomicBool::new(false)),
            cancel_called: Arc::new(AtomicBool::new(false)),
            confirm_count: Arc::new(AtomicUsize::new(0)),
            cancel_count: Arc::new(AtomicUsize::new(0)),
            confirm_result: true,
            cancel_result: true,
        }
    }
}

#[async_trait]
impl PlanConfirmationHandler for MockConfirmHandler {
    async fn confirm(&self, _confirmation_id: &str) -> bool {
        self.confirm_called.store(true, Ordering::SeqCst);
        self.confirm_count.fetch_add(1, Ordering::SeqCst);
        self.confirm_result
    }

    async fn cancel(&self, _confirmation_id: &str) -> bool {
        self.cancel_called.store(true, Ordering::SeqCst);
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
        self.cancel_result
    }
}

// ── CapturingPlugin ──────────────────────────────────────────────────────

struct CapturingPlugin {
    platform: String,
    sent: std::sync::Mutex<Vec<(String, String)>>,
}

impl CapturingPlugin {
    fn new(p: &str) -> Self {
        Self {
            platform: p.into(),
            sent: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn take_sent(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

#[async_trait]
impl IMPlugin for CapturingPlugin {
    fn platform(&self) -> &str {
        &self.platform
    }

    async fn parse_inbound(&self, _: &[u8]) -> Result<Option<NormalizedMessage>, AdapterError> {
        Ok(None)
    }

    fn render(
        &self,
        blocks: &[ContentBlock],
        _: Option<&closeclaw_common::processor::DslParseResult>,
    ) -> RenderedOutput {
        let text = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        RenderedOutput {
            msg_type: "text".into(),
            payload: serde_json::json!({"content": {"text": text}}),
        }
    }

    async fn send(
        &self,
        output: &RenderedOutput,
        peer_id: &str,
        _: Option<&str>,
        _reply_ref: Option<&str>,
    ) -> Result<(), AdapterError> {
        let text = output
            .payload
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        self.sent.lock().unwrap().push((peer_id.to_string(), text));
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    Arc::new(SessionManager::new(
        &make_config(),
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_gw() -> crate::Gateway {
    crate::Gateway::new(make_config(), make_session_manager())
}

async fn install_handler(gw: &crate::Gateway, handler: Arc<dyn PlanConfirmationHandler>) {
    gw.set_plan_confirm_handler(handler).await;
}

async fn install_plugin(gw: &crate::Gateway, plugin: Arc<dyn IMPlugin>) {
    let mut plugins = gw.plugins.write().await;
    plugins.insert(plugin.platform().to_string(), plugin);
}

// ── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_confirm_owner_routes_to_handler() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm abc-123", Some("owner"), "p", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(handler.confirm_called.load(Ordering::SeqCst));
    assert_eq!(handler.confirm_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_cancel_owner_routes_to_handler() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/cancel abc-123", Some("owner"), "p", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(handler.cancel_called.load(Ordering::SeqCst));
    assert_eq!(handler.cancel_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_non_confirm_prefix_returns_none() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "hello world", Some("owner"), "p", "mock")
        .await;

    assert!(result.is_none());
    assert!(!handler.confirm_called.load(Ordering::SeqCst));
    assert!(!handler.cancel_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_confirm_without_id_returns_none() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm  ", Some("owner"), "p", "mock")
        .await;

    assert!(result.is_none());
    assert!(!handler.confirm_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_confirm_empty_string_returns_none() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "", Some("owner"), "p", "mock")
        .await;

    assert!(result.is_none());
}

#[tokio::test]
async fn test_non_owner_confirm_rejected() {
    let gw = make_gw();
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    install_plugin(&gw, plugin.clone()).await;
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm abc", Some("other"), "peer1", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(!handler.confirm_called.load(Ordering::SeqCst));
    let sent = plugin.take_sent();
    assert!(!sent.is_empty());
    assert!(sent[0].1.contains("权限不足"));
}

#[tokio::test]
async fn test_non_owner_cancel_rejected() {
    let gw = make_gw();
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    install_plugin(&gw, plugin.clone()).await;
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/cancel abc", Some("other"), "peer1", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(!handler.cancel_called.load(Ordering::SeqCst));
    let sent = plugin.take_sent();
    assert!(!sent.is_empty());
    assert!(sent[0].1.contains("权限不足"));
}

#[tokio::test]
async fn test_none_sender_confirm_rejected() {
    let gw = make_gw();
    let plugin = Arc::new(CapturingPlugin::new("mock"));
    install_plugin(&gw, plugin.clone()).await;
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm abc", None, "peer1", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(!handler.confirm_called.load(Ordering::SeqCst));
    let sent = plugin.take_sent();
    assert!(!sent.is_empty());
    assert!(sent[0].1.contains("权限不足"));
}

#[tokio::test]
async fn test_no_handler_configured_returns_none() {
    let gw = make_gw();

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm abc", Some("owner"), "p", "mock")
        .await;

    assert!(result.is_none());
}

#[tokio::test]
async fn test_confirm_with_whitespace_id() {
    let gw = make_gw();
    let handler = Arc::new(MockConfirmHandler::new());
    install_handler(&gw, handler.clone()).await;

    let result = gw
        .try_handle_plan_confirm_command("s", "/confirm   id-1", Some("owner"), "p", "mock")
        .await;

    assert!(matches!(result, Some(HandleResult::ApprovalProcessed)));
    assert!(handler.confirm_called.load(Ordering::SeqCst));
}
