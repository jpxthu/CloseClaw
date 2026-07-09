//! Unit tests for manual backgrounding signal mechanism.
//!
//! Covers: trigger_manual_background, manual_background_notify,
//! signal-based tokio::select! in handle_foreground_result, and
//! no-op behavior when no foreground tasks exist.

use std::sync::Arc;
use std::time::Duration;

use super::super::*;

/// trigger_manual_background fires the signal — a waiting notified() resolves.
#[tokio::test]
async fn test_trigger_manual_background_fires_signal() {
    let cs = ConversationSession::new("s1".into(), "m".into(), tmp_path());
    let signal = cs.manual_background_notify();
    let notified = signal.notified();
    cs.trigger_manual_background();
    tokio::time::timeout(Duration::from_millis(100), notified)
        .await
        .expect("trigger_manual_background should fire the signal");
}

/// Double-fire is idempotent — no panic, no error.
#[tokio::test]
async fn test_trigger_manual_background_idempotent() {
    let cs = ConversationSession::new("s2".into(), "m".into(), tmp_path());
    cs.trigger_manual_background();
    cs.trigger_manual_background();
}

/// No foreground tasks registered — trigger is a harmless no-op.
#[tokio::test]
async fn test_trigger_no_foreground_tasks_noop() {
    let cs = ConversationSession::new("s3".into(), "m".into(), tmp_path());
    cs.trigger_manual_background();
    assert!(cs.tool_handles.read().unwrap().is_empty());
}

/// manual_background_notify returns the same Arc across calls.
#[tokio::test]
async fn test_manual_background_notify_returns_same_arc() {
    let cs = ConversationSession::new("s4".into(), "m".into(), tmp_path());
    let s1 = cs.manual_background_notify();
    let s2 = cs.manual_background_notify();
    assert!(Arc::ptr_eq(&s1, &s2));
}

/// Multiple waiters on the same signal are all woken.
#[tokio::test]
async fn test_multiple_waiters_all_woken() {
    let cs = ConversationSession::new("s5".into(), "m".into(), tmp_path());
    let signal = cs.manual_background_notify();

    let n1 = signal.notified();
    let n2 = signal.notified();
    let n3 = signal.notified();

    cs.trigger_manual_background();

    let (r1, r2, r3) = tokio::join!(
        tokio::time::timeout(Duration::from_millis(100), n1),
        tokio::time::timeout(Duration::from_millis(100), n2),
        tokio::time::timeout(Duration::from_millis(100), n3),
    );
    assert!(r1.is_ok(), "first waiter should be woken");
    assert!(r2.is_ok(), "second waiter should be woken");
    assert!(r3.is_ok(), "third waiter should be woken");
}

/// notify_waiters (called by trigger) does NOT wake a future that
/// starts listening AFTER the notify fires.
#[tokio::test]
async fn test_notify_waiters_does_not_park_future() {
    let cs = ConversationSession::new("s6".into(), "m".into(), tmp_path());
    let signal = cs.manual_background_notify();
    cs.trigger_manual_background();
    // A new notified() future started after the notify — should NOT resolve.
    let notified = signal.notified();
    let result = tokio::time::timeout(Duration::from_millis(50), notified).await;
    assert!(
        result.is_err(),
        "notified() started after notify_waiters should not resolve"
    );
}
