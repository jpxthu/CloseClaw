//! Unit tests for `try_handle_approval_command` prefix parsing.
//!
//! Test dimensions:
//! 1. Normal path: `/approve <request_id>` → returns Some(ApprovalProcessed)
//! 2. Normal path: `/deny <request_id>` → returns Some(ApprovalProcessed)
//! 3. Boundary: message without `/approve` or `/deny` prefix → returns None
//! 4. Boundary: `/approve` without request_id → returns None (warn logged)
//! 5. Boundary: empty string → returns None

use std::sync::Arc;

use async_trait::async_trait;
use closeclaw_common::{PendingMessage, PlanState, SessionLookup, SessionMode};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_types::RuleSet;

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
    async fn push_pending_message(
        &self,
        _session_id: &str,
        _msg: PendingMessage,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn get_plan_state(&self, _session_id: &str) -> Option<PlanState> {
        None
    }
    async fn set_plan_state(&self, _session_id: &str, _state: PlanState) {}
    async fn set_session_mode(&self, _session_id: &str, _mode: SessionMode) {}
    async fn set_pending_mode_transition(
        &self,
        _session_id: &str,
        _transition: closeclaw_common::ModeTransition,
    ) {
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> GatewayConfig {
    GatewayConfig {
        name: "test".to_string(),
        rate_limit_per_minute: 100,
        max_message_size: 1024,
        ..Default::default()
    }
}

fn make_session_manager() -> Arc<SessionManager> {
    let config = make_config();
    Arc::new(SessionManager::new(
        &config,
        None,
        None,
        ReasoningLevel::default(),
    ))
}

fn make_gw() -> crate::Gateway {
    let config = make_config();
    let sm = make_session_manager();
    crate::Gateway::new(config, sm)
}

fn noop_notify(_notification: closeclaw_permission::approval_flow::ApprovalNotification) {}
fn noop_whitelist(_agent_id: &str) {}

/// Create a minimal ApprovalFlow suitable for unit tests.
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

/// Install an approval_flow on a Gateway so that approval commands are processed.
async fn install_approval_flow(gw: &crate::Gateway) {
    let flow = make_approval_flow();
    *gw.approval_flow.write().await = Some(Arc::new(tokio::sync::Mutex::new(flow)));
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_approve_command_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "/approve REQ_001", Some("owner"))
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "expected Some(ApprovalProcessed), got {:?}",
        result,
    );
}

#[tokio::test]
async fn test_deny_command_with_request_id() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "/deny REQ_002", Some("owner"))
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "expected Some(ApprovalProcessed), got {:?}",
        result,
    );
}

#[tokio::test]
async fn test_no_prefix_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "hello world", Some("owner"))
        .await;

    assert!(result.is_none(), "expected None for non-approval message");
}

#[tokio::test]
async fn test_approve_without_request_id_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    // /approve with trailing whitespace but no request_id
    let result = gw
        .try_handle_approval_command("session_1", "/approve   ", Some("owner"))
        .await;

    assert!(
        result.is_none(),
        "expected None when /approve has no request_id",
    );
}

#[tokio::test]
async fn test_approve_bare_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    // /approve with absolutely nothing after it
    let result = gw
        .try_handle_approval_command("session_1", "/approve", Some("owner"))
        .await;

    assert!(result.is_none(), "expected None for bare /approve",);
}

#[tokio::test]
async fn test_empty_string_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "", Some("owner"))
        .await;

    assert!(result.is_none(), "expected None for empty string");
}

#[tokio::test]
async fn test_whitespace_only_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "   ", Some("owner"))
        .await;

    assert!(result.is_none(), "expected None for whitespace-only input");
}

#[tokio::test]
async fn test_non_owner_sender_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "/approve REQ_001", Some("other_user"))
        .await;

    assert!(result.is_none(), "expected None when sender is not owner",);
}

#[tokio::test]
async fn test_none_sender_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "/approve REQ_001", None)
        .await;

    assert!(result.is_none(), "expected None when sender is None");
}

#[tokio::test]
async fn test_deny_bare_returns_none() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    let result = gw
        .try_handle_approval_command("session_1", "/deny", Some("owner"))
        .await;

    assert!(result.is_none(), "expected None for bare /deny");
}

#[tokio::test]
async fn test_approve_with_flags_parsed() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    // /approve with --whitelist flag should still parse correctly
    let result = gw
        .try_handle_approval_command("session_1", "/approve REQ_003 --whitelist", Some("owner"))
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "expected Some(ApprovalProcessed) with --whitelist flag, got {:?}",
        result,
    );
}

#[tokio::test]
async fn test_approve_with_extra_args_parsed() {
    let gw = make_gw();
    install_approval_flow(&gw).await;

    // request_id is the first token after /approve
    let result = gw
        .try_handle_approval_command(
            "session_1",
            "/approve REQ_004 --agent-only extra",
            Some("owner"),
        )
        .await;

    assert!(
        matches!(result, Some(HandleResult::ApprovalProcessed)),
        "expected Some(ApprovalProcessed) with extra args, got {:?}",
        result,
    );
}
