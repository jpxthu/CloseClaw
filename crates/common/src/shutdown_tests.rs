//! Unit tests for common-layer ShutdownHandle delegation.
//!
//! Verifies that the common `ShutdownHandle` correctly delegates all
//! trait methods to its inner `dyn ShutdownSignal` without maintaining
//! independent state.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::{ShutdownHandle, ShutdownSignal};

// ── Mock ShutdownSignal ────────────────────────────────────────────────

/// A mock `ShutdownSignal` implementation for testing delegation.
/// Tracks all method calls and internal state for assertions.
struct MockSignal {
    is_shutting_down_val: AtomicBool,
    is_forceful_val: AtomicBool,
    busy_count_val: AtomicUsize,
    escalate_result: AtomicBool,
    increment_count: AtomicUsize,
    decrement_count: AtomicUsize,
}

impl MockSignal {
    fn new() -> Self {
        Self {
            is_shutting_down_val: AtomicBool::new(false),
            is_forceful_val: AtomicBool::new(false),
            busy_count_val: AtomicUsize::new(0),
            escalate_result: AtomicBool::new(true),
            increment_count: AtomicUsize::new(0),
            decrement_count: AtomicUsize::new(0),
        }
    }

    fn with_shutting_down(val: bool) -> Self {
        let m = Self::new();
        m.is_shutting_down_val.store(val, Ordering::SeqCst);
        m
    }

    fn with_forceful(val: bool) -> Self {
        let m = Self::new();
        m.is_forceful_val.store(val, Ordering::SeqCst);
        m
    }

    fn with_busy_count(val: usize) -> Self {
        let m = Self::new();
        m.busy_count_val.store(val, Ordering::SeqCst);
        m
    }

    fn with_escalate_result(val: bool) -> Self {
        let m = Self::new();
        m.escalate_result.store(val, Ordering::SeqCst);
        m
    }
}

impl ShutdownSignal for MockSignal {
    fn is_shutting_down(&self) -> bool {
        self.is_shutting_down_val.load(Ordering::SeqCst)
    }

    fn increment_busy(&self) {
        self.increment_count.fetch_add(1, Ordering::SeqCst);
        self.busy_count_val.fetch_add(1, Ordering::SeqCst);
    }

    fn decrement_busy(&self) {
        self.decrement_count.fetch_add(1, Ordering::SeqCst);
        self.busy_count_val.fetch_sub(1, Ordering::SeqCst);
    }

    fn busy_count(&self) -> usize {
        self.busy_count_val.load(Ordering::SeqCst)
    }

    fn escalate_to_forceful(&self) -> bool {
        self.escalate_result.load(Ordering::SeqCst)
    }

    fn is_forceful(&self) -> bool {
        self.is_forceful_val.load(Ordering::SeqCst)
    }
}

// ── increment_busy / decrement_busy delegation ──────────────────────────

#[test]
fn test_increment_busy_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::new());
    let handle = ShutdownHandle::new(mock.clone());

    handle.increment_busy();
    assert_eq!(mock.increment_count.load(Ordering::SeqCst), 1);
    assert_eq!(mock.busy_count_val.load(Ordering::SeqCst), 1);
}

#[test]
fn test_decrement_busy_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::new());
    let handle = ShutdownHandle::new(mock.clone());

    handle.increment_busy();
    handle.decrement_busy();
    assert_eq!(mock.decrement_count.load(Ordering::SeqCst), 1);
    assert_eq!(mock.busy_count_val.load(Ordering::SeqCst), 0);
}

#[test]
fn test_multiple_increments_decrements_delegates_correctly() {
    let mock = Arc::new(MockSignal::new());
    let handle = ShutdownHandle::new(mock.clone());

    for _ in 0..5 {
        handle.increment_busy();
    }
    assert_eq!(mock.increment_count.load(Ordering::SeqCst), 5);
    assert_eq!(mock.busy_count(), 5);

    for _ in 0..5 {
        handle.decrement_busy();
    }
    assert_eq!(mock.decrement_count.load(Ordering::SeqCst), 5);
    assert_eq!(mock.busy_count(), 0);
}

// ── busy_count delegation ──────────────────────────────────────────────

#[test]
fn test_busy_count_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_busy_count(42));
    let handle = ShutdownHandle::new(mock);

    assert_eq!(handle.busy_count(), 42);
}

#[test]
fn test_busy_count_reflects_inner_signal_changes() {
    let mock = Arc::new(MockSignal::new());
    let handle = ShutdownHandle::new(mock.clone());

    assert_eq!(handle.busy_count(), 0);

    mock.busy_count_val.store(10, Ordering::SeqCst);
    assert_eq!(handle.busy_count(), 10);
}

// ── escalate_to_forceful delegation ────────────────────────────────────

#[test]
fn test_escalate_to_forceful_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_escalate_result(true));
    let handle = ShutdownHandle::new(mock);

    assert!(handle.escalate_to_forceful());
}

#[test]
fn test_escalate_to_forceful_returns_false_when_inner_returns_false() {
    let mock = Arc::new(MockSignal::with_escalate_result(false));
    let handle = ShutdownHandle::new(mock);

    assert!(!handle.escalate_to_forceful());
}

// ── is_shutting_down delegation ────────────────────────────────────────

#[test]
fn test_is_shutting_down_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_shutting_down(true));
    let handle = ShutdownHandle::new(mock);

    assert!(handle.is_shutting_down());
}

#[test]
fn test_is_shutting_down_false_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_shutting_down(false));
    let handle = ShutdownHandle::new(mock);

    assert!(!handle.is_shutting_down());
}

// ── is_forceful delegation ─────────────────────────────────────────────

#[test]
fn test_is_forceful_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_forceful(true));
    let handle = ShutdownHandle::new(mock);

    assert!(handle.is_forceful());
}

#[test]
fn test_is_forceful_false_delegates_to_inner_signal() {
    let mock = Arc::new(MockSignal::with_forceful(false));
    let handle = ShutdownHandle::new(mock);

    assert!(!handle.is_forceful());
}

// ── Clone shares the same inner signal ─────────────────────────────────

#[test]
fn test_clone_shares_inner_signal() {
    let mock = Arc::new(MockSignal::with_busy_count(7));
    let handle1 = ShutdownHandle::new(mock.clone());
    let handle2 = handle1.clone();

    // Both handles should read the same busy_count via the shared mock
    assert_eq!(handle1.busy_count(), 7);
    assert_eq!(handle2.busy_count(), 7);

    // Mutation through one handle is visible through the other
    handle1.increment_busy();
    assert_eq!(handle2.busy_count(), 8);
}

// ── ShutdownSignal impl on ShutdownHandle delegates correctly ──────────

#[test]
fn test_shutdown_signal_trait_impl_delegates_busy_count() {
    let mock = Arc::new(MockSignal::with_busy_count(3));
    let handle = ShutdownHandle::new(mock);
    let signal: &dyn ShutdownSignal = &handle;

    assert_eq!(signal.busy_count(), 3);
}

#[test]
fn test_shutdown_signal_trait_impl_delegates_escalate() {
    let mock = Arc::new(MockSignal::with_escalate_result(true));
    let handle = ShutdownHandle::new(mock);
    let signal: &dyn ShutdownSignal = &handle;

    assert!(signal.escalate_to_forceful());
}

#[test]
fn test_shutdown_signal_trait_impl_delegates_is_forceful() {
    let mock = Arc::new(MockSignal::with_forceful(true));
    let handle = ShutdownHandle::new(mock);
    let signal: &dyn ShutdownSignal = &handle;

    assert!(signal.is_forceful());
}

#[test]
fn test_shutdown_signal_trait_impl_delegates_is_shutting_down() {
    let mock = Arc::new(MockSignal::with_shutting_down(true));
    let handle = ShutdownHandle::new(mock);
    let signal: &dyn ShutdownSignal = &handle;

    assert!(signal.is_shutting_down());
}
