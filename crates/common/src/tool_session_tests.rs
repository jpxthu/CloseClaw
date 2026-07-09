//! Unit tests for ToolSession trait and KillHandle trait.
//!
//! Validates the abstract session registration interface and
//! kill-handle adapters defined in `tool_session`.

use crate::tool_session::{KillHandle, ToolSession};
use async_trait::async_trait;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// =========================================================================
// Mock KillHandle
// =========================================================================

/// A mock kill handle that records whether `kill()` was called.
struct MockKillHandle {
    killed: AtomicBool,
}

impl MockKillHandle {
    fn new() -> Self {
        Self {
            killed: AtomicBool::new(false),
        }
    }

    fn was_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }
}

impl KillHandle for MockKillHandle {
    fn kill(&self) -> io::Result<()> {
        self.killed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// A mock kill handle that returns an error on kill.
struct FailingKillHandle;

impl KillHandle for FailingKillHandle {
    fn kill(&self) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Other, "kill failed"))
    }
}

// =========================================================================
// Mock ToolSession
// =========================================================================

/// A mock ToolSession that records all registered handles.
struct MockToolSession {
    handles: Mutex<Vec<(String, Arc<dyn KillHandle>)>>,
}

impl MockToolSession {
    fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }

    fn registered_handles(&self) -> Vec<(String, Arc<dyn KillHandle>)> {
        self.handles.lock().unwrap().clone()
    }

    fn handle_count(&self) -> usize {
        self.handles.lock().unwrap().len()
    }
}

#[async_trait]
impl ToolSession for MockToolSession {
    async fn register_tool_handle(&self, call_id: String, handle: Arc<dyn KillHandle>) {
        self.handles.lock().unwrap().push((call_id, handle));
    }
}

// =========================================================================
// KillHandle tests
// =========================================================================

#[test]
fn test_kill_handle_records_kill() {
    let handle = MockKillHandle::new();
    assert!(!handle.was_killed());
    handle.kill().unwrap();
    assert!(handle.was_killed());
}

#[test]
fn test_kill_handle_idempotent() {
    let handle = MockKillHandle::new();
    handle.kill().unwrap();
    handle.kill().unwrap();
    assert!(handle.was_killed());
}

#[test]
fn test_failing_kill_handle_returns_error() {
    let handle = FailingKillHandle;
    let result = handle.kill();
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
}

// =========================================================================
// ToolSession tests
// =========================================================================

#[tokio::test]
async fn test_tool_session_register_single_handle() {
    let session = MockToolSession::new();
    let handle: Arc<dyn KillHandle> = Arc::new(MockKillHandle::new());

    session.register_tool_handle("call_1".into(), handle).await;

    assert_eq!(session.handle_count(), 1);
    let handles = session.registered_handles();
    assert_eq!(handles[0].0, "call_1");
}

#[tokio::test]
async fn test_tool_session_register_multiple_handles() {
    let session = MockToolSession::new();

    for i in 0..5 {
        let handle: Arc<dyn KillHandle> = Arc::new(MockKillHandle::new());
        session
            .register_tool_handle(format!("call_{}", i), handle)
            .await;
    }

    assert_eq!(session.handle_count(), 5);
    let handles = session.registered_handles();
    for i in 0..5 {
        assert_eq!(handles[i].0, format!("call_{}", i));
    }
}

#[tokio::test]
async fn test_tool_session_handles_are_killable() {
    let session = MockToolSession::new();
    let mock = Arc::new(MockKillHandle::new());
    let handle: Arc<dyn KillHandle> = Arc::clone(&mock) as Arc<dyn KillHandle>;

    session.register_tool_handle("call_1".into(), handle).await;

    // Kill through the session's registered handle
    let handles = session.registered_handles();
    handles[0].1.kill().unwrap();

    // The original mock should reflect the kill
    assert!(mock.was_killed());
}

#[tokio::test]
async fn test_tool_session_different_call_ids() {
    let session = MockToolSession::new();

    let h1: Arc<dyn KillHandle> = Arc::new(MockKillHandle::new());
    let h2: Arc<dyn KillHandle> = Arc::new(MockKillHandle::new());

    session.register_tool_handle("call_alpha".into(), h1).await;
    session.register_tool_handle("call_beta".into(), h2).await;

    let handles = session.registered_handles();
    assert_eq!(handles[0].0, "call_alpha");
    assert_eq!(handles[1].0, "call_beta");
}

#[tokio::test]
async fn test_tool_session_empty_initially() {
    let session = MockToolSession::new();
    assert_eq!(session.handle_count(), 0);
    assert!(session.registered_handles().is_empty());
}

#[tokio::test]
async fn test_tool_session_with_arc_handles() {
    let session = MockToolSession::new();
    let handle: Arc<dyn KillHandle> = Arc::new(MockKillHandle::new());

    session
        .register_tool_handle("arc_call".into(), handle)
        .await;

    // Verify the Arc is shared (not cloned into the session)
    let handles = session.registered_handles();
    assert_eq!(handles.len(), 1);
    // The Arc should have at least 2 references: our local + the session's
    assert!(Arc::strong_count(&handles[0].1) >= 2);
}

// =========================================================================
// manual_background_notify tests
// =========================================================================

/// Mock ToolSession that returns a Notify signal.
struct MockToolSessionWithSignal {
    handles: Mutex<Vec<(String, Arc<dyn KillHandle>)>>,
    signal: Arc<tokio::sync::Notify>,
}

impl MockToolSessionWithSignal {
    fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
            signal: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[async_trait]
impl ToolSession for MockToolSessionWithSignal {
    async fn register_tool_handle(&self, call_id: String, handle: Arc<dyn KillHandle>) {
        self.handles.lock().unwrap().push((call_id, handle));
    }

    fn manual_background_notify(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(Arc::clone(&self.signal))
    }
}

/// Default trait impl returns None.
#[test]
fn test_tool_session_manual_background_notify_default_returns_none() {
    let session = MockToolSession::new();
    assert!(session.manual_background_notify().is_none());
}

/// Custom impl returns Some(Notify).
#[test]
fn test_tool_session_manual_background_notify_returns_signal() {
    let session = MockToolSessionWithSignal::new();
    assert!(session.manual_background_notify().is_some());
}

/// Signal fires → waiting future completes.
#[tokio::test]
async fn test_manual_background_notify_signal_wakes_waiter() {
    let session = MockToolSessionWithSignal::new();
    let signal = session.manual_background_notify().unwrap();
    // Create a notified future BEFORE firing the signal.
    let notified = signal.notified();
    // Fire the signal — the waiting future should resolve.
    signal.notify_waiters();
    notified.await;
}
