//! Unit tests for SessionManager async setters: `set_output_tx` and `set_gateway_ref`.
//!
//! These tests verify that the setters (which were converted from sync
//! `blocking_write` to async `write().await`) work correctly in an async
//! context without panicking, and that the corresponding getters return
//! the expected values.

use super::tests::{make_test_mgr, test_config};
use crate::types::OutputTx;
use std::sync::Arc;
use tokio::sync::mpsc;

// ── set_output_tx / get_output_tx ───────────────────────────────────────────

/// `set_output_tx` in an async context does not panic and the value
/// is retrievable via `get_output_tx`.
#[tokio::test]
async fn test_set_output_tx_does_not_panic() {
    let mgr = make_test_mgr(None);
    let (tx, _rx) = mpsc::channel(1);
    let output_tx: OutputTx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
    // This must not panic — the original bug was `blocking_write` inside
    // an async fn called from an async context.
    mgr.set_output_tx(output_tx).await;
}

/// After `set_output_tx`, `get_output_tx` returns `Some`.
#[tokio::test]
async fn test_get_output_tx_after_set() {
    let mgr = make_test_mgr(None);
    let (tx, _rx) = mpsc::channel(1);
    let output_tx: OutputTx = Arc::new(tokio::sync::RwLock::new(Some(tx)));
    mgr.set_output_tx(output_tx).await;

    let result = mgr.get_output_tx().await;
    assert!(
        result.is_some(),
        "get_output_tx should return Some after set"
    );
}

/// Before any `set_output_tx` call, `get_output_tx` returns `None`.
#[tokio::test]
async fn test_get_output_tx_before_set_is_none() {
    let mgr = make_test_mgr(None);
    let result = mgr.get_output_tx().await;
    assert!(
        result.is_none(),
        "get_output_tx should return None before set"
    );
}

/// Setting output_tx twice replaces the previous value.
#[tokio::test]
async fn test_set_output_tx_replaces_previous() {
    let mgr = make_test_mgr(None);

    let (tx1, _rx1) = mpsc::channel(1);
    let output_tx1: OutputTx = Arc::new(tokio::sync::RwLock::new(Some(tx1)));
    mgr.set_output_tx(output_tx1).await;

    let (tx2, _rx2) = mpsc::channel(1);
    let output_tx2: OutputTx = Arc::new(tokio::sync::RwLock::new(Some(tx2)));
    mgr.set_output_tx(output_tx2).await;

    let result = mgr.get_output_tx().await;
    assert!(
        result.is_some(),
        "get_output_tx should return Some after second set"
    );
    // The returned Arc should be the second one, not the first.
    // We verify by checking that sending on the second sender works.
    let inner = result.unwrap();
    let guard = inner.read().await;
    assert!(guard.is_some(), "inner sender should be present");
    // Sending should succeed (receiver is alive via _rx2).
    guard
        .as_ref()
        .unwrap()
        .send(("test".into(), vec![]))
        .await
        .unwrap();
}

// ── set_gateway_ref / get_gateway_ref ───────────────────────────────────────

/// `set_gateway_ref` in an async context does not panic and the value
/// is retrievable via `get_gateway_ref`.
#[tokio::test]
async fn test_set_gateway_ref_does_not_panic() {
    let mgr = make_test_mgr(None);
    let session_manager = Arc::new(mgr);
    let gw = Arc::new(crate::Gateway::new_for_tests(
        test_config(),
        Arc::clone(&session_manager),
    ));
    // This must not panic — the original bug was `blocking_write` inside
    // an async fn called from an async context.
    session_manager.set_gateway_ref(Arc::clone(&gw)).await;
}

/// After `set_gateway_ref`, `get_gateway_ref` returns `Some` with a
/// valid strong reference.
#[tokio::test]
async fn test_get_gateway_ref_after_set() {
    let mgr = make_test_mgr(None);
    let session_manager = Arc::new(mgr);
    let gw = Arc::new(crate::Gateway::new_for_tests(
        test_config(),
        Arc::clone(&session_manager),
    ));
    session_manager.set_gateway_ref(Arc::clone(&gw)).await;

    let result = session_manager.get_gateway_ref().await;
    assert!(
        result.is_some(),
        "get_gateway_ref should return Some after set"
    );
    // The returned Arc should point to the same Gateway.
    let retrieved = result.unwrap();
    assert!(
        Arc::ptr_eq(&retrieved, &gw),
        "retrieved Gateway should be the same Arc as the one set"
    );
}

/// Before any `set_gateway_ref` call, `get_gateway_ref` returns `None`.
#[tokio::test]
async fn test_get_gateway_ref_before_set_is_none() {
    let mgr = make_test_mgr(None);
    let result = mgr.get_gateway_ref().await;
    assert!(
        result.is_none(),
        "get_gateway_ref should return None before set"
    );
}

/// After setting a gateway ref, dropping the original `Arc<Gateway>`
/// causes `get_gateway_ref` to return `None` (Weak semantics).
#[tokio::test]
async fn test_get_gateway_ref_returns_none_after_drop() {
    let mgr = make_test_mgr(None);
    let session_manager = Arc::new(mgr);
    let gw = Arc::new(crate::Gateway::new_for_tests(
        test_config(),
        Arc::clone(&session_manager),
    ));
    session_manager.set_gateway_ref(Arc::clone(&gw)).await;

    // Verify it's accessible before drop.
    assert!(
        session_manager.get_gateway_ref().await.is_some(),
        "should be Some before drop"
    );

    // Drop the only strong reference (the one we created).
    drop(gw);

    // The Weak inside session_manager should no longer be upgradeable.
    let result = session_manager.get_gateway_ref().await;
    assert!(
        result.is_none(),
        "get_gateway_ref should return None after the Gateway is dropped"
    );
}

/// Setting gateway_ref twice replaces the previous weak reference.
#[tokio::test]
async fn test_set_gateway_ref_replaces_previous() {
    let mgr = make_test_mgr(None);
    let session_manager = Arc::new(mgr);

    let gw1 = Arc::new(crate::Gateway::new_for_tests(
        test_config(),
        Arc::clone(&session_manager),
    ));
    session_manager.set_gateway_ref(Arc::clone(&gw1)).await;

    let gw2 = Arc::new(crate::Gateway::new_for_tests(
        test_config(),
        Arc::clone(&session_manager),
    ));
    session_manager.set_gateway_ref(Arc::clone(&gw2)).await;

    let result = session_manager.get_gateway_ref().await;
    assert!(result.is_some(), "should return Some after second set");
    let retrieved = result.unwrap();
    assert!(
        Arc::ptr_eq(&retrieved, &gw2),
        "retrieved Gateway should be the second one, not the first"
    );
    // The first gateway should still be alive (held by our local variable).
    assert!(
        Arc::strong_count(&gw1) >= 1,
        "first gateway should still be alive"
    );
}

// ── set_workflow_control / get_workflow_control ─────────────────────────────

/// Minimal `WorkflowControl` stub recording the last invoked signal, so the
/// setter test can pin that the injected `Arc` is the very object stored.
struct RecordingWorkflowControl {
    last_signal: std::sync::Mutex<Vec<(&'static str, String)>>,
}

impl RecordingWorkflowControl {
    fn new() -> Self {
        Self {
            last_signal: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn record(&self, signal: &'static str, session_id: &str) {
        self.last_signal
            .lock()
            .unwrap()
            .push((signal, session_id.to_string()));
    }
}

#[async_trait::async_trait]
impl closeclaw_common::WorkflowControl for RecordingWorkflowControl {
    async fn start(&self, session_id: &str, _workflow_name: &str) -> Result<(), String> {
        self.record("start", session_id);
        Ok(())
    }

    async fn on_owner_message(&self, session_id: &str) -> Result<(), String> {
        self.record("on_owner_message", session_id);
        Ok(())
    }

    async fn terminate(&self, session_id: &str) -> Result<(), String> {
        self.record("terminate", session_id);
        Ok(())
    }
}

/// Before any `set_workflow_control` call, `get_workflow_control` returns `None`.
#[tokio::test]
async fn test_get_workflow_control_before_set_is_none() {
    let mgr = make_test_mgr(None);
    let result = mgr.get_workflow_control().await;
    assert!(
        result.is_none(),
        "get_workflow_control should return None before set"
    );
}

/// After `set_workflow_control`, `get_workflow_control` returns `Some` pointing
/// to the same object (same allocation), and the trait object is callable
/// through the retrieved handle.
#[tokio::test]
async fn test_get_workflow_control_after_set() {
    let mgr = make_test_mgr(None);
    let control = Arc::new(RecordingWorkflowControl::new());
    mgr.set_workflow_control(Arc::clone(&control) as Arc<dyn closeclaw_common::WorkflowControl>)
        .await;

    let retrieved = mgr
        .get_workflow_control()
        .await
        .expect("get_workflow_control should return Some after set");
    // Same allocation (data-pointer identity across the dyn coat).
    assert_eq!(
        Arc::as_ptr(&control) as *const u8,
        Arc::as_ptr(&retrieved) as *const u8,
        "retrieved control should be the same object as the one set"
    );
    // Behavior pin: the retrieved handle routes into the same recorder.
    retrieved.start("session-1", "demo").await.unwrap();
    retrieved.on_owner_message("session-2").await.unwrap();
    retrieved.terminate("session-3").await.unwrap();
    let recorded = control.last_signal.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec![
            ("start", "session-1".to_string()),
            ("on_owner_message", "session-2".to_string()),
            ("terminate", "session-3".to_string()),
        ]
    );
}

/// Setting `workflow_control` twice replaces the previous value: only the
/// second control receives subsequent signals.
#[tokio::test]
async fn test_set_workflow_control_replaces_previous() {
    let mgr = make_test_mgr(None);
    let first = Arc::new(RecordingWorkflowControl::new());
    let second = Arc::new(RecordingWorkflowControl::new());
    mgr.set_workflow_control(Arc::clone(&first) as Arc<dyn closeclaw_common::WorkflowControl>)
        .await;
    mgr.set_workflow_control(Arc::clone(&second) as Arc<dyn closeclaw_common::WorkflowControl>)
        .await;

    let retrieved = mgr.get_workflow_control().await.expect("should be set");
    retrieved.terminate("session-9").await.unwrap();
    assert!(
        first.last_signal.lock().unwrap().is_empty(),
        "replaced control must not receive signals"
    );
    assert_eq!(
        second.last_signal.lock().unwrap().clone(),
        vec![("terminate", "session-9".to_string())],
    );
}
