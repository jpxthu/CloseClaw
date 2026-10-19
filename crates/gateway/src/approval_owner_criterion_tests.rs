//! Step 1.5 — approval Owner criterion dimension tests (checkpoint
//! `get_sender_id` 口径统一后的行为维度覆盖).
//!
//! Split from `approval_tests.rs` (file-length budget) — dimensions per
//! plan Step 1.5:
//! ① checkpoint 无 Owner（`sender_id = None`）→ 审批命令一律拒绝；
//! ② sender ≠ 会话 Owner → 审批拒绝（非字面 Owner id 复验）；
//! ③ terminal uid 会话（sender = uid = checkpoint Owner）→ 真实 Owner
//!    审批放行（无拒绝回复）。
//!
//! (ownerless-session / literal-sender slash and engine counterparts live
//! in `tests_slash_permission.rs` and `engine_owner_tests.rs`.)

use std::sync::Arc;

use async_trait::async_trait;

use crate::gateway_test_utils::SpyPlugin;
use crate::slash_permission_test_utils::make_gateway_with_owner;
use crate::HandleResult;
use closeclaw_common::{PendingMessage, PlanState, SessionLookup, SessionMode};
use closeclaw_permission::approval_flow::{ApprovalFlow, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_types::RuleSet;

// ── Mocks ────────────────────────────────────────────────────────────────────

struct LookupMock;

#[async_trait]
impl SessionLookup for LookupMock {
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

fn noop_notify(_: closeclaw_permission::approval_flow::ApprovalNotification) {}
fn noop_whitelist(_: &str) {}

/// Install an `ApprovalFlow` on `gw` backed by the caller-held config
/// dir (STANDARDS §8 — the caller owns the `TempDir` lifetime).
async fn install_approval_flow_at(gw: &crate::Gateway, config_dir: &std::path::Path) {
    let flow = ApprovalFlow::new(
        Arc::new(LookupMock) as Arc<dyn SessionLookup>,
        Arc::new(noop_notify),
        Arc::new(noop_whitelist),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        config_dir.to_path_buf(),
        RuleSet::default(),
    );
    *gw.approval_flow.write().await = Some(Arc::new(tokio::sync::Mutex::new(flow)));
}

// ═════════════════════════════════════════════════════════════════════════════
// Dimension ① — checkpoint 无 Owner（sender_id = None）：一律拒绝
// ═════════════════════════════════════════════════════════════════════════════

/// Sessions without a bound Owner grant approval to nobody: every
/// sender — including the literal `"owner"` id — is rejected at the
/// Owner gate and receives the "权限不足" reply.
#[tokio::test]
async fn test_no_bound_owner_rejects_any_approval_sender() {
    let spy = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-noowner", None).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    let dir = tempfile::tempdir().expect("approval config dir");
    install_approval_flow_at(&gw, dir.path()).await;

    for sender in [Some("owner"), Some("ou_someone"), None] {
        let r = gw
            .try_handle_approval_command("sess-noowner", "/deny REQ_020", sender, "peer", "mock")
            .await;
        assert!(
            matches!(r, Some(HandleResult::ApprovalProcessed)),
            "sender {sender:?}: approval command must be consumed as rejected, got {:?}",
            r
        );
    }

    let msgs = spy.lock().expect("spy poisoned");
    assert_eq!(
        msgs.len(),
        3,
        "each attempt must receive exactly one rejection reply: {:?}",
        *msgs
    );
    assert!(
        msgs.iter().all(|m| m.contains("权限不足")),
        "every rejection must carry the denial text: {:?}",
        *msgs
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Dimension ② — sender ≠ 会话 Owner：审批拒绝（非字面 Owner id 复验）
// ═════════════════════════════════════════════════════════════════════════════

/// With a non-literal checkpoint Owner bound, a different sender is
/// rejected (the non-owner series in `approval_tests.rs` covers the
/// bound-`"owner"` shape; this re-verifies the criterion with a
/// platform-style Owner id).
#[tokio::test]
async fn test_non_literal_bound_owner_rejects_other_sender() {
    let spy = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-two", Some("ou_session_owner")).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    let dir = tempfile::tempdir().expect("approval config dir");
    install_approval_flow_at(&gw, dir.path()).await;

    let r = gw
        .try_handle_approval_command(
            "sess-two",
            "/deny REQ_021",
            Some("someone_else"),
            "peer",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "non-owner /deny must be consumed as rejected, got {:?}",
        r
    );

    let msgs = spy.lock().expect("spy poisoned");
    assert_eq!(
        msgs.len(),
        1,
        "exactly one rejection reply expected: {:?}",
        *msgs
    );
    assert!(
        msgs[0].contains("权限不足"),
        "rejection must carry the denial text, got: {:?}",
        msgs[0]
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Dimension ③ — terminal uid 会话（sender = uid = checkpoint Owner）放行
// ═════════════════════════════════════════════════════════════════════════════

/// Terminal-channel sessions bind the numeric OS uid as the checkpoint
/// Owner (`platform/src/terminal.rs::current_uid`). A sender equal to
/// that uid is the real Owner: the `/deny` command passes the Owner
/// gate, reaches the ApprovalFlow (consumed as `ApprovalProcessed`),
/// and no rejection reply is sent.
#[tokio::test]
async fn test_terminal_uid_owner_approval_allowed() {
    let spy = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-uid", Some("1000")).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    let dir = tempfile::tempdir().expect("approval config dir");
    install_approval_flow_at(&gw, dir.path()).await;

    let r = gw
        .try_handle_approval_command("sess-uid", "/deny REQ_022", Some("1000"), "peer", "mock")
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "uid Owner /deny must be processed by the approval flow, got {:?}",
        r
    );

    let msgs = spy.lock().expect("spy poisoned");
    assert!(
        !msgs.iter().any(|m| m.contains("权限不足")),
        "the real Owner must NOT receive a rejection reply, got: {:?}",
        *msgs
    );
}

/// Guard for the dimension-③ assertion: the same command from a
/// different sender IS rejected — proving the empty-rejection assertion
/// above reflects the Owner gate passing, not a silently broken
/// outbound path.
#[tokio::test]
async fn test_terminal_uid_session_still_rejects_other_sender() {
    let spy = Arc::new(std::sync::Mutex::new(Vec::new()));
    let gw = make_gateway_with_owner("sess-uid-neg", Some("1000")).await;
    gw.register_plugin(Arc::new(SpyPlugin::new(Arc::clone(&spy))))
        .await;
    let dir = tempfile::tempdir().expect("approval config dir");
    install_approval_flow_at(&gw, dir.path()).await;

    let r = gw
        .try_handle_approval_command(
            "sess-uid-neg",
            "/deny REQ_023",
            Some("2000"),
            "peer",
            "mock",
        )
        .await;
    assert!(
        matches!(r, Some(HandleResult::ApprovalProcessed)),
        "non-owner /deny must be consumed, got {:?}",
        r
    );

    let msgs = spy.lock().expect("spy poisoned");
    assert_eq!(msgs.len(), 1, "exactly one rejection expected: {:?}", *msgs);
    assert!(msgs[0].contains("权限不足"));
}
