//! Unit tests for `try_handle_approval_command` prefix parsing and
//! approval command interception move-up (Step 1.3).
//!
//! Test dimensions:
//! 1. Normal path: `/approve <request_id>` → returns Some(ApprovalProcessed)
//! 2. Normal path: `/deny <request_id>` → returns Some(ApprovalProcessed)
//! 3. Boundary: message without `/approve` or `/deny` prefix → returns None
//! 4. Boundary: `/approve` without request_id → returns None (warn logged)
//! 5. Boundary: empty string → returns None
//! 6. Non-owner sender + `/approve` → returns Some(ApprovalProcessed) + rejection message
//! 7. None sender + `/approve` → returns Some(ApprovalProcessed) + rejection message
//! 8. Non-owner sender + `/deny` → returns Some(ApprovalProcessed) + rejection message
//! 9. Non-owner sender + `/approve` without request_id → returns Some(ApprovalProcessed)
//!    (permission check precedes request_id parsing)
//!
//! Step 1.3 — approval command interception move-up:
//! 10. Approval command processed without SlashDispatcher
//! 11. Busy session /deny → immediate (not queued)
//! 12. Busy session non-approval slash → queued via dispatcher
//! 13. Non-owner /deny → "权限不足"
//! 14. Busy session /approve-once → immediate (not queued)
//! 15. Idle session non-approval slash → dispatcher executes

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::im_plugin::{AdapterError, MessageType, NormalizedMessage, RenderedOutput};
use closeclaw_common::processor::{ContentBlock, ProcessedMessage};
use closeclaw_common::slash_router::{SlashContext, SlashHandler, SlashResult, SlashRouter};
use closeclaw_common::{IMPlugin, PendingMessage, PlanState, SessionLookup, SessionMode};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_types::RuleSet;

use crate::inbound_queue::InboundDebugCtx;
use crate::{GatewayConfig, HandleResult, SessionManager};
use closeclaw_session::persistence::ReasoningLevel;

// ── Local mock of SessionLookup (permission crate's mock is #[cfg(test)] only)

struct MockLookup;

#[async_trait]
impl SessionLookup for MockLookup {
    async fn get_parent_of(&self, _child_id: &str) -> Option<String> {
        None
    }
    async fn get_chat_id(&self, _session_id: &str) -> Option<String> {
        None
    }
    async fn push_pending_message(&self, _: &str, _: PendingMessage) -> Result<(), String> {
        Ok(())
    }
    async fn get_plan_state(&self, _: &str) -> Option<PlanState> {
        None
    }
    async fn set_plan_state(&self, _: &str, _: PlanState) {}
    async fn set_session_mode(&self, _: &str, _: SessionMode) {}
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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

fn noop_notify(_n: closeclaw_permission::approval_flow::ApprovalNotification) {}
fn noop_whitelist(_: &str) {}

fn make_approval_flow() -> ApprovalFlow {
    let mock_lookup: Arc<dyn SessionLookup> = Arc::new(MockLookup);
    let handle = tokio::runtime::Handle::current();
    let config_dir = tempfile::tempdir().unwrap().keep();
    ApprovalFlow::new(
        mock_lookup,
        Arc::new(noop_notify),
        Arc::new(noop_whitelist),
        handle,
        HeartbeatApprovalMode::default(),
        config_dir,
        RuleSet::default(),
    )
}

async fn install_approval_flow(gw: &crate::Gateway) {
    *gw.approval_flow.write().await = Some(Arc::new(tokio::sync::Mutex::new(make_approval_flow())));
}

/// Captures messages sent via IMPlugin::send for assertion.
/// Used in approval tests to verify outbound notifications and rejection messages.
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
        _: Option<&str>,
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

// ── Slash infrastructure for Step 1.3 tests ─────────────────────────────────

struct S13Handler {
    cmd: String,
}
#[async_trait]
impl SlashHandler for S13Handler {
    fn commands(&self) -> &[&str] {
        &[]
    }
    fn description(&self) -> &str {
        "test"
    }
    fn requires_permission(&self) -> bool {
        false
    }
    fn clone_box(&self) -> Box<dyn SlashHandler> {
        Box::new(S13Handler {
            cmd: self.cmd.clone(),
        })
    }
    async fn handle(&self, _: &str, _: &SlashContext) -> SlashResult {
        SlashResult::Reply(format!("handled:{}", self.cmd))
    }
}

struct S13Router;
#[async_trait]
impl SlashRouter for S13Router {
    async fn dispatch(&self, _: &str, _: &SlashContext) -> Option<SlashResult> {
        None
    }
    fn is_immediate(&self, cmd: &str) -> bool {
        cmd == "help"
    }
    fn get_handler(&self, cmd: &str) -> Option<Box<dyn SlashHandler>> {
        match cmd {
            "help" | "compact" => Some(Box::new(S13Handler {
                cmd: cmd.to_owned(),
            })),
            _ => None,
        }
    }
}

fn s13_cfg() -> GatewayConfig {
    GatewayConfig {
        name: "s13".into(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

async fn s13_env(
    sid: &str,
    ch: &str,
    p: Arc<dyn IMPlugin>,
) -> (Arc<crate::Gateway>, Arc<SessionManager>) {
    let sm = Arc::new(SessionManager::new(
        &s13_cfg(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        sid.into(),
        crate::Session {
            id: sid.into(),
            agent_id: "a".into(),
            channel: ch.into(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = crate::Gateway::new(s13_cfg(), Arc::clone(&sm));
    gw.register_plugin(p).await;
    gw.set_slash_dispatcher(Arc::new(S13Router)).await;
    (Arc::new(gw), sm)
}

async fn s13_busy(sm: &SessionManager, sid: &str) {
    let cs = closeclaw_session::llm_session::ConversationSession::new(
        sid.into(),
        "m".into(),
        std::path::PathBuf::from("/tmp"),
    );
    cs.set_llm_state(closeclaw_common::LlmState::Requesting);
    sm.conversation_sessions
        .write()
        .await
        .insert(sid.into(), Arc::new(tokio::sync::RwLock::new(cs)));
}

fn s13_msg(text: &str, peer: &str, sender: &str) -> ProcessedMessage {
    let mut m = std::collections::HashMap::new();
    m.insert("peer_id".into(), peer.into());
    m.insert("sender_id".into(), sender.into());
    m.insert(
        "message_type".into(),
        serde_json::to_string(&MessageType::Text).unwrap(),
    );
    m.insert("session_key".into(), "k".into());
    ProcessedMessage {
        content_blocks: vec![ContentBlock::Text(text.into())],
        metadata: m,
    }
}

fn s13_dbg() -> InboundDebugCtx<'static> {
    InboundDebugCtx {
        trace_id: None,
        session_key: None,
        root_ctx: None,
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests — prefix parsing
// ═════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_approve_command_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve REQ_001", Some("owner"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_deny_command_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/deny REQ_002", Some("owner"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_no_prefix_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "hello", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_approve_without_request_id_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve   ", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_approve_bare_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_empty_string_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_whitespace_only_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "   ", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_non_owner_sender_returns_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve REQ_001", Some("other"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_non_owner_deny_returns_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/deny REQ_002", Some("other"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_non_owner_approve_without_request_id_returns_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve", Some("other"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_none_sender_returns_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve REQ_001", None, "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_none_sender_deny_returns_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/deny REQ_002", None, "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_deny_bare_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/deny", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_approve_with_flags_parsed() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve REQ_003 --whitelist",
            Some("owner"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approve_with_extra_args_parsed() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve REQ_004 --agent-only extra",
            Some("owner"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approve_once_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve-once REQ_010", Some("owner"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approve_whitelist_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve-whitelist REQ_011",
            Some("owner"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approve_whitelist_agent_only() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve-whitelist REQ_012 --agent-only",
            Some("owner"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approve_whitelist_user_and_agent() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve-whitelist REQ_013 --user-and-agent",
            Some("owner"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_approval_prefix_no_match() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approval REQ_014", Some("owner"), "p", "mock")
        .await;
    assert!(r.is_none());
}

#[tokio::test]
async fn test_deny_once_matches_deny_prefix() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/deny-once REQ_015", Some("owner"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_non_owner_approve_once_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s", "/approve-once REQ_016", Some("other"), "p", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

#[tokio::test]
async fn test_non_owner_approve_whitelist_rejected() {
    let gw = make_gw();
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command(
            "s",
            "/approve-whitelist REQ_017",
            Some("other"),
            "p",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Step 1.3 — Approval command interception move-up tests
// ═════════════════════════════════════════════════════════════════════════════

/// Approval command processed without SlashDispatcher.
#[tokio::test]
async fn test_approval_without_slash_dispatcher() {
    let sm = Arc::new(SessionManager::new(
        &s13_cfg(),
        None,
        None,
        ReasoningLevel::default(),
    ));
    sm.sessions.write().await.insert(
        "s1".into(),
        crate::Session {
            id: "s1".into(),
            agent_id: "a".into(),
            channel: "mock".into(),
            created_at: 0,
            depth: 0,
        },
    );
    let gw = crate::Gateway::new(s13_cfg(), Arc::clone(&sm));
    gw.register_plugin(Arc::new(CapturingPlugin::new("mock")))
        .await;
    install_approval_flow(&gw).await;
    // No slash_dispatcher set — approval must still work.
    let gw = Arc::new(gw);
    let r = gw
        .handle_inbound_message(
            s13_msg("/deny REQ_001", "peer1", "owner"),
            Some("owner"),
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "/deny must work without SlashDispatcher, got {:?}",
        r
    );
}

/// Busy session /deny → immediate, not queued.
#[tokio::test]
async fn test_busy_deny_immediate() {
    let (gw, sm) = s13_env("s2", "mock", Arc::new(CapturingPlugin::new("mock"))).await;
    install_approval_flow(&gw).await;
    s13_busy(&sm, "s2").await;
    let r = gw
        .route_and_dispatch(
            &s13_msg("/deny REQ_002", "p", "o"),
            "s2",
            "/deny REQ_002".into(),
            Some("owner"),
            "peer2",
            "mock",
            &s13_dbg(),
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "busy /deny should process immediately, got {:?}",
        r
    );
    assert!(
        sm.pop_pending_message("s2").await.is_none(),
        "approval should not be enqueued"
    );
}

/// Busy session non-approval slash → queued.
#[tokio::test]
async fn test_busy_slash_queued() {
    let (gw, sm) = s13_env("s3", "mock", Arc::new(CapturingPlugin::new("mock"))).await;
    s13_busy(&sm, "s3").await;
    let r = gw
        .route_and_dispatch(
            &s13_msg("/compact", "p", "o"),
            "s3",
            "/compact".into(),
            Some("owner"),
            "peer3",
            "mock",
            &s13_dbg(),
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::SlashHandled)),
        "busy /compact should go through dispatcher, got {:?}",
        r
    );
    let pending = sm.pop_pending_message("s3").await;
    assert!(pending.is_some(), "/compact should be enqueued when busy");
    assert!(pending.unwrap().content.contains("/compact"));
}

/// Non-owner /deny → "权限不足".
#[tokio::test]
async fn test_non_owner_deny_rejection() {
    let p = Arc::new(CapturingPlugin::new("mock"));
    let p_ref = Arc::clone(&p);
    let (gw, _) = s13_env("s4", "mock", p).await;
    install_approval_flow(&gw).await;
    let r = gw
        .try_handle_approval_command("s4", "/deny REQ_004", Some("user"), "peer4", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "got {:?}",
        r
    );
    assert!(
        p_ref
            .take_sent()
            .iter()
            .any(|(_, t)| t.contains("权限不足")),
        "should receive rejection"
    );
}

/// Busy session /approve-once → immediate, not queued.
#[tokio::test]
async fn test_busy_approve_once_immediate() {
    let (gw, sm) = s13_env("s5", "mock", Arc::new(CapturingPlugin::new("mock"))).await;
    install_approval_flow(&gw).await;
    s13_busy(&sm, "s5").await;
    let r = gw
        .route_and_dispatch(
            &s13_msg("/approve-once REQ_005", "p", "o"),
            "s5",
            "/approve-once REQ_005".into(),
            Some("owner"),
            "peer5",
            "mock",
            &s13_dbg(),
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "busy /approve-once should process immediately, got {:?}",
        r
    );
    assert!(
        sm.pop_pending_message("s5").await.is_none(),
        "/approve-once should not be enqueued"
    );
}

/// Idle session non-approval slash → dispatcher executes.
#[tokio::test]
async fn test_idle_slash_executes() {
    let p = Arc::new(CapturingPlugin::new("mock"));
    let p_ref = Arc::clone(&p);
    let (gw, sm) = s13_env("s6", "mock", p).await;
    s13_busy(&sm, "s6").await;
    let cs = sm
        .conversation_sessions
        .read()
        .await
        .get("s6")
        .cloned()
        .unwrap();
    cs.write()
        .await
        .set_llm_state(closeclaw_common::LlmState::Idle);
    let r = gw
        .route_and_dispatch(
            &s13_msg("/compact", "p", "o"),
            "s6",
            "/compact".into(),
            Some("owner"),
            "peer6",
            "mock",
            &s13_dbg(),
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::SlashHandled)),
        "idle /compact should be handled, got {:?}",
        r
    );
    assert!(
        p_ref
            .take_sent()
            .iter()
            .any(|(_, t)| t.contains("handled:compact")),
        "handler reply should be sent"
    );
}
