//! Step 1.5 — Child session busy tracking tests.
//!
//! Verifies:
//! - `register_child_handle` increments busy_count
//! - `unregister_child_handle` decrements busy_count
//! - Multiple register/unregister cycles track correctly
//! - Shutdown gate rejects new child sessions
//! - Full lifecycle: register → stop → clear_exec_state resets count

use super::super::*;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_common::ShutdownSignal;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock as TokioRwLock;

fn make_session(id: &str) -> Arc<TokioRwLock<ConversationSession>> {
    Arc::new(TokioRwLock::new(ConversationSession::new(
        id.to_string(),
        "gpt-4o".to_string(),
        tmp_path(),
    )))
}

/// Minimal mock for child-session busy tracking tests.
struct ChildTrackingMock {
    shutting_down: AtomicBool,
    busy: AtomicUsize,
}
impl ChildTrackingMock {
    fn new() -> Self {
        Self {
            shutting_down: AtomicBool::new(false),
            busy: AtomicUsize::new(0),
        }
    }
    fn set_shutting_down(&self, v: bool) {
        self.shutting_down.store(v, Ordering::SeqCst);
    }
}
impl ShutdownSignal for ChildTrackingMock {
    fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }
    fn increment_busy(&self) {
        self.busy.fetch_add(1, Ordering::SeqCst);
    }
    fn decrement_busy(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
    }
    fn busy_count(&self) -> usize {
        self.busy.load(Ordering::SeqCst)
    }
    fn escalate_to_forceful(&self) -> bool {
        false
    }
    fn is_forceful(&self) -> bool {
        false
    }
    fn drain_status(&self) -> closeclaw_common::DrainStatus {
        closeclaw_common::DrainStatus {
            state: closeclaw_common::shutdown::ShutdownState::Running,
            busy_count: self.busy.load(Ordering::SeqCst),
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
}

/// register_child_handle increments busy_count.
#[tokio::test]
async fn test_register_child_handle_increments_busy_count() {
    let cs = make_session("s_child_busy_inc");
    let sh = Arc::new(ChildTrackingMock::new());
    {
        let mut g = cs.write().await;
        g.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }
    let child = make_session("child_inc");
    assert_eq!(sh.busy_count(), 0);
    cs.read()
        .await
        .register_child_handle("child_inc", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 1);
}

/// unregister_child_handle decrements busy_count.
#[tokio::test]
async fn test_unregister_child_handle_decrements_busy_count() {
    let cs = make_session("s_child_busy_dec");
    let sh = Arc::new(ChildTrackingMock::new());
    {
        let mut g = cs.write().await;
        g.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }
    let child = make_session("child_dec");
    cs.read()
        .await
        .register_child_handle("child_dec", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 1);
    cs.read().await.unregister_child_handle("child_dec");
    assert_eq!(sh.busy_count(), 0);
}

/// Multiple register/unregister: busy_count tracks net registrations.
#[tokio::test]
async fn test_child_handle_multi_register_unregister() {
    let cs = make_session("s_child_busy_multi");
    let sh = Arc::new(ChildTrackingMock::new());
    {
        let mut g = cs.write().await;
        g.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }
    let (c1, c2, c3) = (
        make_session("mc1"),
        make_session("mc2"),
        make_session("mc3"),
    );
    cs.read()
        .await
        .register_child_handle("mc1", Arc::downgrade(&c1));
    assert_eq!(sh.busy_count(), 1);
    cs.read()
        .await
        .register_child_handle("mc2", Arc::downgrade(&c2));
    assert_eq!(sh.busy_count(), 2);
    cs.read()
        .await
        .register_child_handle("mc3", Arc::downgrade(&c3));
    assert_eq!(sh.busy_count(), 3);
    cs.read().await.unregister_child_handle("mc1");
    cs.read().await.unregister_child_handle("mc3");
    assert_eq!(sh.busy_count(), 1);
    cs.read().await.unregister_child_handle("mc2");
    assert_eq!(sh.busy_count(), 0);
}

/// Shutdown gate: register_child_handle rejected when shutting down.
#[tokio::test]
async fn test_register_child_handle_rejected_during_shutdown() {
    let cs = make_session("s_child_gate");
    let sh = Arc::new(ChildTrackingMock::new());
    {
        let mut g = cs.write().await;
        g.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }
    sh.set_shutting_down(true);
    let child = make_session("child_rejected");
    assert_eq!(sh.busy_count(), 0);
    cs.read()
        .await
        .register_child_handle("child_rejected", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 0, "must NOT increment when shutting down");
    assert!(cs
        .read()
        .await
        .child_handles
        .read()
        .expect("lock")
        .is_empty());
}

/// Busy count tracked through register → stop → clear_exec_state.
#[tokio::test]
async fn test_child_busy_count_through_stop_lifecycle() {
    let cs = make_session("s_child_lc");
    let sh = Arc::new(ChildTrackingMock::new());
    {
        let mut g = cs.write().await;
        g.set_shutdown_handle(sh.clone() as Arc<dyn ShutdownSignal>);
    }
    let child = make_session("child_lc");
    cs.read()
        .await
        .register_child_handle("child_lc", Arc::downgrade(&child));
    assert_eq!(sh.busy_count(), 1);
    cs.read()
        .await
        .stop(true, ShutdownMode::Forceful, Duration::ZERO)
        .await;
    assert_eq!(
        sh.busy_count(),
        0,
        "stop must clear handles and reset busy_count"
    );
    assert!(child.read().await.is_stopped());
}
