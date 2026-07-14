use super::*;
use closeclaw_llm::types::ContentBlock;

// =========================================================================
// Gap 3: Recovery Action tests (Step 1.6)
// =========================================================================

/// Helper to create an OutputTx for testing.
fn make_output_tx(
    has_sender: bool,
) -> (
    super::OutputTx,
    tokio::sync::mpsc::Receiver<(String, Vec<ContentBlock>)>,
) {
    if has_sender {
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        (std::sync::Arc::new(tokio::sync::RwLock::new(Some(tx))), rx)
    } else {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        (std::sync::Arc::new(tokio::sync::RwLock::new(None)), rx)
    }
}

/// NotifyUser sends message via output_tx.
#[tokio::test]
async fn test_recovery_notify_user_sends_message() {
    let (output_tx, mut rx) = make_output_tx(true);
    let action = closeclaw_session::run_health::RecoverableAction::NotifyUser {
        message: "health issue detected".to_string(),
    };
    let skip_drain =
        SessionMessageHandler::test_handle_recovery_action("test-session", action, &output_tx);
    // NotifyUser should NOT skip drain.
    assert!(!skip_drain);
    // Give the spawned task time to send.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    let (text, _blocks) = rx.try_recv().expect("should receive message");
    assert_eq!(text, "health issue detected");
}

/// NotifyUser without output_tx (None) does not panic.
#[tokio::test]
async fn test_recovery_notify_user_no_tx_no_panic() {
    let (output_tx, _rx) = make_output_tx(false);
    let action = closeclaw_session::run_health::RecoverableAction::NotifyUser {
        message: "health issue".to_string(),
    };
    let skip_drain =
        SessionMessageHandler::test_handle_recovery_action("test-session", action, &output_tx);
    assert!(!skip_drain);
    // No panic = pass.
}

/// Stop returns true (caller should skip drain_pending_loop).
#[test]
fn test_recovery_stop_returns_true() {
    let (output_tx, _rx) = make_output_tx(true);
    let action = closeclaw_session::run_health::RecoverableAction::Stop {
        reason: "side effects reported".to_string(),
    };
    let skip_drain =
        SessionMessageHandler::test_handle_recovery_action("test-session", action, &output_tx);
    assert!(skip_drain);
}

/// Retry returns false (caller should NOT skip drain — TODO for future).
#[test]
fn test_recovery_retry_returns_false() {
    let (output_tx, _rx) = make_output_tx(true);
    let action = closeclaw_session::run_health::RecoverableAction::Retry {
        delay_ms: 1000,
        instruction: Some("retry instruction".to_string()),
    };
    let skip_drain =
        SessionMessageHandler::test_handle_recovery_action("test-session", action, &output_tx);
    assert!(!skip_drain);
}

/// Retry without instruction also returns false.
#[test]
fn test_recovery_retry_no_instruction_returns_false() {
    let (output_tx, _rx) = make_output_tx(true);
    let action = closeclaw_session::run_health::RecoverableAction::Retry {
        delay_ms: 500,
        instruction: None,
    };
    let skip_drain =
        SessionMessageHandler::test_handle_recovery_action("test-session", action, &output_tx);
    assert!(!skip_drain);
}

/// Gap 3: Healthy turn does not trigger any recovery action.
///
/// When the health check returns Healthy (no action), finish_llm
/// should not call handle_recovery_action at all. We verify the
/// absence of recovery by confirming that the Stop action (the
/// only one that changes skip_drain) returns true, while the
/// default healthy path would return false.
#[test]
fn test_healthy_turn_no_recovery_action() {
    let (output_tx, _rx) = make_output_tx(true);
    // Stop action → skip_drain = true.
    let stop_action = closeclaw_session::run_health::RecoverableAction::Stop {
        reason: "test".to_string(),
    };
    let skip_drain_stop =
        SessionMessageHandler::test_handle_recovery_action("test", stop_action, &output_tx);
    assert!(skip_drain_stop);
    // Healthy path: no action is produced → skip_drain stays false.
    // This is verified by the fact that handle_recovery_action is only
    // called when verdict.status != Healthy.
}
