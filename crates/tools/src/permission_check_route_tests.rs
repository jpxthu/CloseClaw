//! Behavior tests for the denial-routing helpers (`route_denial` /
//! `route_command_denial`), kept in a separate module file so that
//! `permission_check_tests.rs` stays within the 1000-line limit.

use super::tests::{make_af_deny, make_sm};
use super::*;
use closeclaw_permission::approval_flow::{ApprovalNotification, HeartbeatApprovalMode};
use closeclaw_permission::engine::engine_types::RuleSet;
use std::sync::Mutex as StdMutex;

// ---------------------------------------------------------------------------
// route_denial / route_command_denial behavior tests (Step 1.2)
//
// These exercise the denial-routing helpers directly with hand-built
// `PermissionResponse` values, covering four observable dimensions:
// accept → approval-pending result, existing request id → short-circuit,
// rejected flow → hard denial, non-Denied response → untouched flow.
// ---------------------------------------------------------------------------

/// Standard approval flow with a capturing owner-notification callback so
/// tests can observe whether (and how often) the flow was contacted.
fn make_af_capturing() -> (Arc<ApprovalMutex>, Arc<StdMutex<Vec<ApprovalNotification>>>) {
    let notifications: Arc<StdMutex<Vec<ApprovalNotification>>> = Arc::default();
    let sink = Arc::clone(&notifications);
    let flow = Arc::new(TokioMutex::new(ApprovalFlow::new(
        Arc::clone(&make_sm()) as Arc<dyn closeclaw_common::SessionLookup>,
        Arc::new(move |n: ApprovalNotification| {
            sink.lock().unwrap().push(n);
        }),
        Arc::new(|_: &str| {}),
        tokio::runtime::Handle::current(),
        HeartbeatApprovalMode::default(),
        std::env::temp_dir(),
        RuleSet::default(),
    )));
    (flow, notifications)
}

/// Build a `Denied` response, optionally carrying an existing approval
/// request id (simulating a denial already submitted by the engine).
fn denied_response(approval_request_id: Option<String>) -> PR {
    // Production `check_*` copies this risk level into the `DenialRequest`,
    // so the accept-path tests pass `High` when building the request and
    // assert it propagates into the owner notification.
    PR::Denied {
        reason: "no matching allow rule".to_string(),
        rule: "test-rule".to_string(),
        risk_level: RiskLevel::High,
        approval_request_id,
    }
}

/// Bundle a denial submission context (root session, not a sub-agent).
fn make_denial_request<'a>(
    caller: &'a Caller,
    body: &'a PermissionRequestBody,
    session_id: &'a str,
) -> DenialRequest<'a> {
    DenialRequest::new(caller, body, RiskLevel::Low, session_id, false)
}

/// Normal path: `Denied` + flow accepts → `Ok(Some(result))` whose `data`
/// is exactly the approval-pending payload carrying the submitted request id.
#[tokio::test]
async fn test_route_denial_flow_accept_returns_approval_pending() {
    let (flow, notifications) = make_af_capturing();
    let caller = Caller {
        user_id: "ou_owner".to_string(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "read".to_string(),
    };
    let response = denied_response(None);

    let result = route_denial(
        &response,
        DenialRequest::new(&caller, &body, RiskLevel::High, "sess-1", false),
        &flow,
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
    let caller = Caller {
        user_id: "ou_owner".to_string(),
        agent: "agent-sub".to_string(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "agent-sub".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    };
    let response = denied_response(None);
    let request = DenialRequest::new(&caller, &body, RiskLevel::High, "sess-sub", true);

    let result = route_denial(&response, request, &flow).await;
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
        &response,
        DenialRequest::new(&caller, &body, RiskLevel::High, "sess-sub", true),
        &flow,
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
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "write".to_string(),
    };
    let response = denied_response(Some("req-e2e-existing".to_string()));

    let result = route_denial(&response, make_denial_request(&caller, &body, ""), &flow)
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
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::ToolCall {
        agent: "agent-a".to_string(),
        skill: "bash".to_string(),
        method: "call".to_string(),
    };
    let response = denied_response(None);

    let result = route_denial(&response, make_denial_request(&caller, &body, ""), &flow).await;
    match result {
        Err(ToolCallError::PermissionDenied(reason)) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected PermissionDenied, got {:?}", other),
    }
}

/// Boundary: non-Denied (Allowed) response → `Ok(None)` and the approval
/// flow is never contacted.
#[tokio::test]
async fn test_route_denial_allowed_response_is_noop() {
    let (flow, notifications) = make_af_capturing();
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::FileOp {
        agent: "agent-a".to_string(),
        path: "/tmp/test.txt".to_string(),
        op: "read".to_string(),
    };
    let response = PR::Allowed {
        token: "tok".to_string(),
        context_modifier: None,
    };

    let result = route_denial(&response, make_denial_request(&caller, &body, ""), &flow).await;
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
    let caller = Caller {
        user_id: "ou_owner".to_string(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "rm".to_string(),
        args: vec!["-rf".to_string(), "/".to_string()],
    };
    let response = denied_response(None);

    let result = route_command_denial(
        &response,
        DenialRequest::new(&caller, &body, RiskLevel::High, "sess-2", false),
        &flow,
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
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "echo".to_string(),
        args: vec![],
    };
    let response = denied_response(Some("req-e2e-cmd".to_string()));

    let result =
        route_command_denial(&response, make_denial_request(&caller, &body, ""), &flow).await;
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
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "rm".to_string(),
        args: vec!["-rf".to_string(), "/".to_string()],
    };
    let response = denied_response(None);

    let result =
        route_command_denial(&response, make_denial_request(&caller, &body, ""), &flow).await;
    match result {
        CommandPermissionResult::Denied(reason) => {
            assert_eq!(reason, "no matching allow rule");
        }
        other => panic!("expected Denied, got {:?}", other),
    }
}

/// Boundary (command): non-Denied (Allowed) response → `Permitted`, flow
/// untouched.
#[tokio::test]
async fn test_route_command_denial_allowed_response_is_permitted() {
    let (flow, notifications) = make_af_capturing();
    let caller = Caller {
        user_id: String::new(),
        agent: "agent-a".to_string(),
    };
    let body = PermissionRequestBody::CommandExec {
        agent: "agent-a".to_string(),
        cmd: "echo".to_string(),
        args: vec![],
    };
    let response = PR::Allowed {
        token: "tok".to_string(),
        context_modifier: None,
    };

    let result =
        route_command_denial(&response, make_denial_request(&caller, &body, ""), &flow).await;
    assert!(
        matches!(result, CommandPermissionResult::Permitted),
        "Allowed → Permitted"
    );
    assert!(
        notifications.lock().unwrap().is_empty(),
        "approval flow must not be touched for Allowed responses"
    );
}
