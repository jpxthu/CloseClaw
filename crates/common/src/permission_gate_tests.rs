//! Unit tests for the permission DI traits and shared handle aliases.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::*;
use crate::permission_types::RiskLevel;

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

fn sample_request() -> PermissionRequest {
    PermissionRequest {
        agent_id: "agent-main".into(),
        user_id: "ou_caller".into(),
        is_sub_session: false,
        session_id: Some("sess-001".into()),
        operation: PermissionOperation::FileRead {
            path: "/workspace/notes.md".into(),
        },
        purpose: Some("read session notes".into()),
    }
}

fn sample_card(approval_no: u64) -> ApprovalCard {
    ApprovalCard {
        approval_no: Some(approval_no),
        request_id: None,
        kind: ApprovalKind::HostExecution,
        agent_id: Some("agent-main".into()),
        user_id: Some("ou_caller".into()),
        operation: Some(PermissionOperation::Command {
            command: "cargo build".into(),
            location: ExecLocation::Host,
        }),
        risk_level: RiskLevel::High,
        purpose: Some("build the workspace".into()),
        security_note: None,
        network_targets: None,
        allowed_decisions: vec![ApprovalDecisionKind::Once],
        usage_hint: format!("/approve {approval_no} once"),
        deadline: "2026-11-04T23:00:00Z".into(),
    }
}

struct CannedGate {
    decision: PermissionDecision,
    received: Mutex<Vec<PermissionRequest>>,
}

impl CannedGate {
    fn received(&self) -> Vec<PermissionRequest> {
        self.received.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl PermissionGate for CannedGate {
    async fn evaluate(&self, request: PermissionRequest) -> PermissionDecision {
        self.received.lock().unwrap().push(request.clone());
        self.decision.clone()
    }
}

#[derive(Default)]
struct RecordingSandboxExecutor {
    last_policy: Mutex<Option<ExecutionPolicy>>,
    last_command: Mutex<Option<(String, Duration)>>,
    reject_file_operation: bool,
}

impl RecordingSandboxExecutor {
    fn last_policy(&self) -> Option<ExecutionPolicy> {
        self.last_policy.lock().unwrap().clone()
    }

    fn last_command(&self) -> Option<(String, Duration)> {
        self.last_command.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl SandboxExecutor for RecordingSandboxExecutor {
    async fn execute_file_operation(
        &self,
        policy: &ExecutionPolicy,
        operation: Box<dyn FnOnce() -> io::Result<()> + Send>,
    ) -> io::Result<()> {
        *self.last_policy.lock().unwrap() = Some(policy.clone());
        if self.reject_file_operation {
            // Simulates the OS boundary rejecting before the body runs.
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path outside sandbox boundary",
            ));
        }
        operation()
    }

    async fn execute_command(
        &self,
        policy: &ExecutionPolicy,
        command: &str,
        timeout: Duration,
    ) -> io::Result<std::process::Output> {
        *self.last_policy.lock().unwrap() = Some(policy.clone());
        *self.last_command.lock().unwrap() = Some((command.to_string(), timeout));
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: b"built\n".to_vec(),
            stderr: Vec::new(),
        })
    }
}

#[derive(Default)]
struct RecordingApprovalNotifier {
    delivered: Mutex<Vec<ApprovalCard>>,
    refreshed: Mutex<Vec<Vec<ApprovalCard>>>,
    next_result: Option<ApprovalDelivery>,
}

impl RecordingApprovalNotifier {
    fn delivered(&self) -> Vec<ApprovalCard> {
        self.delivered.lock().unwrap().clone()
    }

    fn refreshed(&self) -> Vec<Vec<ApprovalCard>> {
        self.refreshed.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl ApprovalNotifier for RecordingApprovalNotifier {
    async fn deliver(&self, card: ApprovalCard) -> ApprovalDelivery {
        self.delivered.lock().unwrap().push(card);
        self.next_result.unwrap_or(ApprovalDelivery::Delivered)
    }

    async fn refresh(&self, cards: Vec<ApprovalCard>) {
        self.refreshed.lock().unwrap().push(cards);
    }
}

// ---------------------------------------------------------------------------
// Object safety
// ---------------------------------------------------------------------------

#[test]
fn test_permission_di_trait_handles_are_object_safe_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SharedPermissionGate>();
    assert_send_sync::<SharedSandboxExecutor>();
    assert_send_sync::<SharedApprovalNotifier>();
}

// ---------------------------------------------------------------------------
// PermissionGate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_shared_permission_gate_alias_returns_denied_decision() {
    let gate: SharedPermissionGate = Arc::new(CannedGate {
        decision: PermissionDecision::Denied {
            reason: "operation denied by permission rules".into(),
            risk_level: RiskLevel::High,
        },
        received: Mutex::new(Vec::new()),
    });
    let decision = gate.evaluate(sample_request()).await;
    match decision {
        PermissionDecision::Denied { reason, risk_level } => {
            assert_eq!(reason, "operation denied by permission rules");
            assert_eq!(risk_level, RiskLevel::High);
        }
        other => panic!("expected a denial, got {other:?}"),
    }
}

#[tokio::test]
async fn test_shared_permission_gate_alias_returns_allowed_with_policy() {
    let policy = ExecutionPolicy {
        location: ExecLocation::Sandbox,
        read_paths: vec!["/workspace".into()],
        write_paths: vec!["/workspace/out".into()],
        deny_paths: vec![],
        network_targets: vec![],
    };
    let gate: SharedPermissionGate = Arc::new(CannedGate {
        decision: PermissionDecision::Allowed {
            policy: Some(policy.clone()),
        },
        received: Mutex::new(Vec::new()),
    });
    let decision = gate.evaluate(sample_request()).await;
    assert_eq!(
        decision,
        PermissionDecision::Allowed {
            policy: Some(policy)
        }
    );
}

/// The caller's request reaches the implementation unchanged (the shared
/// handle is a transparent passthrough, not a rewriter).
#[tokio::test]
async fn test_shared_permission_gate_passes_request_to_implementation_unchanged() {
    let mock = Arc::new(CannedGate {
        decision: PermissionDecision::Allowed { policy: None },
        received: Mutex::new(Vec::new()),
    });
    let gate: SharedPermissionGate = Arc::clone(&mock) as SharedPermissionGate;
    let request = sample_request();
    let decision = gate.evaluate(request).await;
    assert_eq!(decision, PermissionDecision::Allowed { policy: None });
    let received = mock.received();
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0],
        sample_request(),
        "the caller's request reaches the implementation unchanged"
    );
}

/// The approval wait completes inside `evaluate` — the caller is held until
/// the conclusion arrives (no polling, no callbacks). The wait is driven by
/// a oneshot channel so the assertions are deterministic (no fixed sleeps).
#[tokio::test]
async fn test_permission_gate_approval_wait_completes_inside_evaluate() {
    struct WaitingGate {
        entered_tx: mpsc::Sender<()>,
        conclusion_rx: Mutex<Option<oneshot::Receiver<PermissionDecision>>>,
    }

    #[async_trait::async_trait]
    impl PermissionGate for WaitingGate {
        async fn evaluate(&self, _request: PermissionRequest) -> PermissionDecision {
            self.entered_tx
                .try_send(())
                .expect("test still waits for the entered signal");
            let conclusion_rx = self
                .conclusion_rx
                .lock()
                .unwrap()
                .take()
                .expect("conclusion channel set once");
            conclusion_rx.await.expect("test sends the conclusion")
        }
    }

    let (entered_tx, mut entered_rx) = mpsc::channel(1);
    let (conclusion_tx, conclusion_rx) = oneshot::channel();
    let gate: SharedPermissionGate = Arc::new(WaitingGate {
        entered_tx,
        conclusion_rx: Mutex::new(Some(conclusion_rx)),
    });

    let handle = tokio::spawn({
        let gate = Arc::clone(&gate);
        async move { gate.evaluate(sample_request()).await }
    });
    entered_rx
        .recv()
        .await
        .expect("evaluate entered and now waits for the Owner's conclusion");
    assert!(
        !handle.is_finished(),
        "evaluate must not return before the impl-produced conclusion"
    );
    conclusion_tx
        .send(PermissionDecision::Allowed { policy: None })
        .expect("evaluate is still waiting for the conclusion");
    let decision = handle
        .await
        .expect("evaluate completes once the conclusion arrives");
    assert_eq!(decision, PermissionDecision::Allowed { policy: None });
}

// ---------------------------------------------------------------------------
// SandboxExecutor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_shared_sandbox_executor_runs_file_operation_body() {
    let mock = Arc::new(RecordingSandboxExecutor::default());
    let executor: SharedSandboxExecutor = Arc::clone(&mock) as SharedSandboxExecutor;
    let policy = ExecutionPolicy {
        location: ExecLocation::Sandbox,
        read_paths: vec!["/workspace".into()],
        ..Default::default()
    };
    // Read results are stored into the body's captured state.
    let contents = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&contents);
    executor
        .execute_file_operation(
            &policy,
            Box::new(move || {
                sink.lock().unwrap().push_str("sandboxed read");
                Ok(())
            }),
        )
        .await
        .unwrap();
    assert_eq!(&*contents.lock().unwrap(), "sandboxed read");
    assert_eq!(
        mock.last_policy().unwrap().read_paths,
        vec!["/workspace".to_string()],
        "the executor must receive the policy bound to this execution"
    );
}

#[tokio::test]
async fn test_sandbox_executor_boundary_error_propagates_and_body_never_runs() {
    let executor: SharedSandboxExecutor = Arc::new(RecordingSandboxExecutor {
        reject_file_operation: true,
        ..Default::default()
    });
    let policy = ExecutionPolicy::default();
    let body_ran = Arc::new(Mutex::new(false));
    let ran = Arc::clone(&body_ran);
    let error = executor
        .execute_file_operation(
            &policy,
            Box::new(move || {
                *ran.lock().unwrap() = true;
                Ok(())
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        !*body_ran.lock().unwrap(),
        "out-of-boundary access is rejected by the OS boundary before the body runs"
    );
}

#[tokio::test]
async fn test_shared_sandbox_executor_runs_command_with_policy_binding() {
    let mock = Arc::new(RecordingSandboxExecutor::default());
    let executor: SharedSandboxExecutor = Arc::clone(&mock) as SharedSandboxExecutor;
    let policy = ExecutionPolicy {
        location: ExecLocation::Sandbox,
        read_paths: vec!["/workspace".into()],
        network_targets: vec![],
        ..Default::default()
    };
    let output = executor
        .execute_command(&policy, "cargo build", Duration::from_secs(30))
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"built\n");
    assert!(output.stderr.is_empty());
    let (command, timeout) = mock.last_command().unwrap();
    assert_eq!(command, "cargo build");
    assert_eq!(timeout, Duration::from_secs(30));
    let bound_policy = mock.last_policy().unwrap();
    assert_eq!(bound_policy.location, ExecLocation::Sandbox);
    assert!(
        bound_policy.network_targets.is_empty(),
        "sandboxed processes never have network capability"
    );
}

// ---------------------------------------------------------------------------
// ApprovalNotifier
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_shared_approval_notifier_delivers_once_without_retry() {
    let mock = Arc::new(RecordingApprovalNotifier::default());
    let notifier: SharedApprovalNotifier = Arc::clone(&mock) as SharedApprovalNotifier;
    let delivery = notifier.deliver(sample_card(42)).await;
    assert_eq!(delivery, ApprovalDelivery::Delivered);
    assert_eq!(mock.delivered().len(), 1);
    let delivered = mock.delivered();
    assert_eq!(delivered[0].approval_no, Some(42));
    assert_eq!(delivered[0].usage_hint, "/approve 42 once");
}

#[tokio::test]
async fn test_shared_approval_notifier_delivery_result_flows_through() {
    let notifier: SharedApprovalNotifier = Arc::new(RecordingApprovalNotifier {
        next_result: Some(ApprovalDelivery::NoChannel),
        ..Default::default()
    });
    let delivery = notifier.deliver(sample_card(7)).await;
    assert_eq!(delivery, ApprovalDelivery::NoChannel);
}

#[tokio::test]
async fn test_shared_approval_notifier_refresh_covers_all_pending_cards() {
    let mock = Arc::new(RecordingApprovalNotifier::default());
    let notifier: SharedApprovalNotifier = Arc::clone(&mock) as SharedApprovalNotifier;
    let pending = vec![sample_card(1), sample_card(2)];
    notifier.refresh(pending.clone()).await;
    assert_eq!(mock.refreshed(), vec![pending]);
}
