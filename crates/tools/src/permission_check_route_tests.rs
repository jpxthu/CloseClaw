//! Behavior tests for the denial-routing helpers (`route_denial` /
//! `route_command_denial`), kept in a separate module file so that
//! `permission_check_tests.rs` stays within the 1000-line limit.

use super::tests::{
    make_af_capturing, make_af_deny, make_cm, make_engine_with_rules, make_port, make_sm,
};
use super::*;
use closeclaw_permission::engine::engine_risk::RiskLevel;

// ---------------------------------------------------------------------------
// route_denial / route_command_denial behavior tests (Step 1.2)
//
// These exercise the denial-routing helpers directly with hand-built
// `PermVerdict` values, covering four observable dimensions:
// accept → approval-pending result, existing request id → short-circuit,
// rejected flow → hard denial, non-Denied verdict → untouched flow.
// ---------------------------------------------------------------------------

/// Build a `Denied` verdict, optionally carrying an existing approval
/// request id (simulating a denial already submitted by the engine).
fn denied_verdict(approval_request_id: Option<String>) -> PermVerdict {
    // Production `check_*` copies this risk level into the `DenialRequest`,
    // so the accept-path tests pass `High` when building the request and
    // assert it propagates into the owner notification.
    PermVerdict::Denied {
        reason: "no matching allow rule".to_string(),
        risk_level: PermRiskLevel::High,
        approval_request_id,
    }
}

/// Bundle a denial submission context (root session, not a sub-agent).
fn make_denial_request<'a>(
    caller: &'a PermCaller,
    body: &'a PermRequestBody,
    session_id: &'a str,
) -> DenialRequest<'a> {
    DenialRequest::new(caller, body, PermRiskLevel::Low, session_id, false)
}

/// Payload path for `FileOp` bodies, derived from the system temp dir
/// (never a hardcoded `/tmp` literal).
fn temp_file_path() -> String {
    std::env::temp_dir()
        .join("test.txt")
        .to_str()
        .unwrap()
        .to_owned()
}

/// Normal path: `Denied` + flow accepts → `Ok(Some(result))` whose `data`
/// is exactly the approval-pending payload carrying the submitted request id.
#[tokio::test]
async fn test_route_denial_flow_accept_returns_approval_pending() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: "ou_owner".to_string(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: temp_file_path(),
        op: "read".to_string(),
    };
    let verdict = denied_verdict(None);

    let result = route_denial(
        &verdict,
        DenialRequest::new(&caller, &body, PermRiskLevel::High, "sess-1", false),
        &port,
    )
    .await
    .expect("flow accepts → Ok");
    let result = result.expect("Denied without existing id → Some(approval-pending)");
    assert_eq!(result.data["status"], "approval_pending");
    assert!(result.new_messages.is_empty());
    assert!(result.context_modifier.is_none());

    let notes = notifications.lock().unwrap();
    assert_eq!(notes.len(), 1, "flow should be contacted exactly once");
    assert_eq!(
        result.data,
        approval_utils::build_approval_pending(notes[0].request_id.clone()),
        "data is the approval-pending payload for the submitted request"
    );
    assert_eq!(notes[0].caller.user_id, "ou_owner");
    assert_eq!(
        notes[0].risk_level,
        RiskLevel::High,
        "risk_level from the response must propagate to the owner notification"
    );
}

/// Sub-agent path: `is_sub_agent: true` → `submit_denial` silently rejects,
/// so the routing helpers surface a hard denial without notifying anyone.
#[tokio::test]
async fn test_route_denial_sub_agent_is_silently_denied() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: "ou_owner".to_string(),
        agent: "agent-sub".to_string(),
    };
    let body = PermRequestBody::FileOp {
        agent: "agent-sub".to_string(),
        path: temp_file_path(),
        op: "write".to_string(),
    };
    let verdict = denied_verdict(None);
    let request = DenialRequest::new(&caller, &body, PermRiskLevel::High, "sess-sub", true);

    let result = route_denial(&verdict, request, &port).await;
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
    assert!(
        notifications.lock().unwrap().is_empty(),
        "sub-agent denials must be silent: no owner notification"
    );

    // Same contract at the command level: hard denial, no notification.
    let result = route_command_denial(
        &verdict,
        DenialRequest::new(&caller, &body, PermRiskLevel::High, "sess-sub", true),
        &port,
    )
    .await;
    match result {
        CommandPermissionResult::Denied(reason) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected Denied, got {:?}", other),
    }
    assert!(
        notifications.lock().unwrap().is_empty(),
        "sub-agent denials must be silent: no owner notification"
    );
}

/// Short-circuit path: `Denied` already carrying an approval_request_id →
/// returned as-is with that id; the flow is never re-contacted (verified via
/// the capturing callback: zero notifications, i.e. no duplicate submission).
#[tokio::test]
async fn test_route_denial_existing_request_id_short_circuits() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: temp_file_path(),
        op: "write".to_string(),
    };
    let verdict = denied_verdict(Some("req-e2e-existing".to_string()));

    let result = route_denial(&verdict, make_denial_request(&caller, &body, ""), &port)
        .await
        .expect("short-circuit → Ok")
        .expect("short-circuit still returns approval-pending result");
    assert_eq!(result.data["status"], "approval_pending");
    assert_eq!(result.data["request_id"], "req-e2e-existing");
    assert!(
        notifications.lock().unwrap().is_empty(),
        "existing request id must not re-submit to the approval flow"
    );
}

/// Error path: flow rejects the submission → `PermissionDenied(reason)`.
#[tokio::test]
async fn test_route_denial_flow_rejected_returns_permission_denied() {
    let flow = make_af_deny();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::ToolCall {
        agent: "agent-a".to_string(),
        skill: "bash".to_string(),
        method: "call".to_string(),
    };
    let verdict = denied_verdict(None);

    let result = route_denial(&verdict, make_denial_request(&caller, &body, ""), &port).await;
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

/// Boundary: non-Denied (Allowed) verdict → `Ok(None)` and the approval
/// flow is never contacted.
#[tokio::test]
async fn test_route_denial_allowed_response_is_noop() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: temp_file_path(),
        op: "read".to_string(),
    };
    let verdict = PermVerdict::Allowed;

    let result = route_denial(&verdict, make_denial_request(&caller, &body, ""), &port).await;
    assert!(matches!(result, Ok(None)), "Allowed → Ok(None)");
    assert!(
        notifications.lock().unwrap().is_empty(),
        "approval flow must not be touched for Allowed responses"
    );
}

/// Normal path (command): `Denied` + flow accepts → `PendingApproval` whose
/// `data` is exactly the approval-pending payload for the submitted request.
#[tokio::test]
async fn test_route_command_denial_flow_accept_returns_pending() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: "ou_owner".to_string(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "rm".to_string(),
        args: vec!["-rf".to_string(), "/".to_string()],
    };
    let verdict = denied_verdict(None);

    let result = route_command_denial(
        &verdict,
        DenialRequest::new(&caller, &body, PermRiskLevel::High, "sess-2", false),
        &port,
    )
    .await;
    let result = match result {
        CommandPermissionResult::PendingApproval(result) => result,
        other => panic!("expected PendingApproval, got {:?}", other),
    };
    assert_eq!(result.data["status"], "approval_pending");
    let notes = notifications.lock().unwrap();
    assert_eq!(notes.len(), 1, "flow should be contacted exactly once");
    assert_eq!(
        result.data,
        approval_utils::build_approval_pending(notes[0].request_id.clone()),
        "data is the approval-pending payload for the submitted request"
    );
    assert_eq!(
        notes[0].risk_level,
        RiskLevel::High,
        "risk_level from the denial must propagate to the owner notification"
    );
}

/// Short-circuit path (command): existing approval_request_id is echoed back
/// without contacting the flow (zero captured notifications).
#[tokio::test]
async fn test_route_command_denial_existing_request_id_short_circuits() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "echo".to_string(),
        args: vec![],
    };
    let verdict = denied_verdict(Some("req-e2e-cmd".to_string()));

    let result =
        route_command_denial(&verdict, make_denial_request(&caller, &body, ""), &port).await;
    match result {
        CommandPermissionResult::PendingApproval(result) => {
            assert_eq!(result.data["status"], "approval_pending");
            assert_eq!(result.data["request_id"], "req-e2e-cmd");
        }
        other => panic!("expected PendingApproval, got {:?}", other),
    }
    assert!(
        notifications.lock().unwrap().is_empty(),
        "existing request id must not re-submit to the approval flow"
    );
}

/// Error path (command): flow rejects → `Denied(reason)` for sandboxing.
#[tokio::test]
async fn test_route_command_denial_flow_rejected_returns_denied() {
    let flow = make_af_deny();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "rm".to_string(),
        args: vec!["-rf".to_string(), "/".to_string()],
    };
    let verdict = denied_verdict(None);

    let result =
        route_command_denial(&verdict, make_denial_request(&caller, &body, ""), &port).await;
    match result {
        CommandPermissionResult::Denied(reason) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected Denied, got {:?}", other),
    }
}

/// Boundary (command): non-Denied (Allowed) verdict → `Permitted`, flow
/// untouched.
#[tokio::test]
async fn test_route_command_denial_allowed_response_is_permitted() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "echo".to_string(),
        args: vec![],
    };
    let verdict = PermVerdict::Allowed;

    let result =
        route_command_denial(&verdict, make_denial_request(&caller, &body, ""), &port).await;
    assert!(
        matches!(result, CommandPermissionResult::Permitted),
        "Allowed → Permitted"
    );
    assert!(
        notifications.lock().unwrap().is_empty(),
        "approval flow must not be touched for Allowed responses"
    );
}

// ---------------------------------------------------------------------------
// approval_utils::approval_pending construction tests
//
// The bash trust-level path (`trust.rs`, `TrustDecision::ApprovalPending`)
// and the denial-routing path (`route_denial_core`) both embed the
// `ToolResult` built by this single helper, so pinning its construction
// output here pins the pending-result isomorphism between the two paths
// (the trust side is additionally covered end-to-end by
// `bash_approval_tests.rs`).
// ---------------------------------------------------------------------------

/// Normal path: `approval_pending` wraps exactly the three-key payload
/// from `build_approval_pending`, with no injected agent messages and no
/// context modifier — the "wait for owner approval" response contract.
#[test]
fn test_approval_pending_constructs_exact_pending_result() {
    let result = approval_utils::approval_pending("req-ut-pending".to_string());
    assert_eq!(
        result.data,
        approval_utils::build_approval_pending("req-ut-pending".to_string()),
        "data is the shared payload builder output verbatim"
    );
    assert_eq!(
        result.data,
        serde_json::json!({
            "status": "approval_pending",
            "request_id": "req-ut-pending",
            "message": "Operation pending owner approval",
        }),
        "payload contract: exactly status / request_id / message"
    );
    assert!(result.new_messages.is_empty(), "no injected agent messages");
    assert!(result.context_modifier.is_none(), "no context modifier");
}

/// Consistency: the pending `ToolResult` the routing path hands back is
/// field-isomorphic to the shared helper's output for the same request id
/// (payload, no messages, no modifier). `trust.rs` embeds the same helper
/// call in `TrustDecision::ApprovalPending`, so this pins both paths to
/// one construction.
#[tokio::test]
async fn test_route_pending_result_isomorphic_to_approval_pending_helper() {
    let (flow, notifications) = make_af_capturing();
    let port = make_port(make_engine_with_rules(vec![]), make_sm(), make_cm(), flow);
    let caller = PermCaller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: temp_file_path(),
        op: "write".to_string(),
    };
    let verdict = denied_verdict(Some("req-ut-iso".to_string()));

    let result = route_denial(&verdict, make_denial_request(&caller, &body, ""), &port)
        .await
        .expect("short-circuit → Ok")
        .expect("short-circuit → pending result");

    let helper = approval_utils::approval_pending("req-ut-iso".to_string());
    assert_eq!(
        result.data, helper.data,
        "routing path embeds the shared helper's payload"
    );
    assert!(
        result.new_messages.is_empty() && helper.new_messages.is_empty(),
        "neither the routing path nor the helper injects agent messages"
    );
    assert!(
        result.context_modifier.is_none() && helper.context_modifier.is_none(),
        "neither the routing path nor the helper sets a context modifier"
    );
    assert!(
        notifications.lock().unwrap().is_empty(),
        "isomorphism ran on the short-circuit path: flow never re-contacted"
    );
}
