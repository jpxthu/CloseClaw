//! Unit tests for `ShutdownHandle` (migrated from `closeclaw_common::shutdown`).
//!
//! Covers:
//! - Normal path: create handle → trigger shutdown → verify state transitions
//!   (Running → Draining → Complete)
//! - Error path: idempotency of repeated shutdown triggers
//! - Boundary value: `DrainStatus` snapshot accuracy
//! - State transition: `ShutdownMode` (Graceful vs Immediate) behavioral impact

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

use closeclaw_common::shutdown::{DrainStatus, ShutdownMode, ShutdownSignal, ShutdownState};

use crate::shutdown_handle::ShutdownHandle;

// ── Mock infrastructure ──────────────────────────────────────────────────────

/// A mock `ShutdownSignal` that simulates realistic state transitions.
///
/// The mock tracks an internal `ShutdownState` and `busy_count`. Calling
/// `trigger_shutdown()` advances the state machine, and `complete_drain()`
/// finishes the drain when `busy_count == 0`.
struct MockTransitionSignal {
    state: AtomicU8,
    busy_count: AtomicUsize,
    is_forceful: AtomicBool,
}

impl MockTransitionSignal {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(ShutdownState::Running as u8),
            busy_count: AtomicUsize::new(0),
            is_forceful: AtomicBool::new(false),
        }
    }

    fn get_state(&self) -> ShutdownState {
        ShutdownState::from_u8(self.state.load(Ordering::SeqCst))
    }

    /// Simulate a shutdown signal being received.
    fn trigger_shutdown(&self, mode: ShutdownMode) {
        match mode {
            ShutdownMode::Graceful => {
                self.state
                    .store(ShutdownState::Draining as u8, Ordering::SeqCst);
            }
            ShutdownMode::Forceful => {
                self.is_forceful.store(true, Ordering::SeqCst);
                self.state
                    .store(ShutdownState::ForcefulShuttingDown as u8, Ordering::SeqCst);
            }
        }
    }

    /// Simulate drain completion when busy_count reaches 0.
    fn complete_drain(&self) {
        if self.get_state() == ShutdownState::Draining
            && self.busy_count.load(Ordering::SeqCst) == 0
        {
            self.state
                .store(ShutdownState::Stopped as u8, Ordering::SeqCst);
        }
    }
}

impl ShutdownSignal for MockTransitionSignal {
    fn is_shutting_down(&self) -> bool {
        self.get_state().is_shutting_down_state()
    }

    fn increment_busy(&self) {
        self.busy_count.fetch_add(1, Ordering::SeqCst);
    }

    fn decrement_busy(&self) {
        // Saturate at 0 to avoid underflow panic.
        let prev = self.busy_count.load(Ordering::SeqCst);
        if prev == 0 {
            return;
        }
        let new_val = self.busy_count.fetch_sub(1, Ordering::SeqCst);
        // Auto-complete drain if busy_count drops to 0 while draining.
        if new_val == 1 {
            self.complete_drain();
        }
    }

    fn busy_count(&self) -> usize {
        self.busy_count.load(Ordering::SeqCst)
    }

    fn escalate_to_forceful(&self) -> bool {
        let current = self.get_state();
        if matches!(
            current,
            ShutdownState::ShuttingDown | ShutdownState::Draining | ShutdownState::Stopped
        ) {
            self.is_forceful.store(true, Ordering::SeqCst);
            self.state
                .store(ShutdownState::ForcefulShuttingDown as u8, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn is_forceful(&self) -> bool {
        self.is_forceful.load(Ordering::SeqCst)
    }

    fn drain_status(&self) -> DrainStatus {
        let state = self.get_state();
        DrainStatus {
            state,
            busy_count: self.busy_count.load(Ordering::SeqCst),
            is_draining: state == ShutdownState::Draining,
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Normal path: create → trigger shutdown → verify state transitions
// ═════════════════════════════════════════════════════════════════════════════

/// Create a handle, trigger graceful shutdown, verify Running → Draining.
#[test]
fn test_graceful_shutdown_transitions_to_draining() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    assert!(
        !handle.is_shutting_down(),
        "should start as not shutting down"
    );
    assert_eq!(handle.busy_count(), 0);

    signal.trigger_shutdown(ShutdownMode::Graceful);

    assert!(handle.is_shutting_down());
    let status = handle.drain_status();
    assert_eq!(status.state, ShutdownState::Draining);
    assert!(status.is_draining);
}

/// Drain completes: busy_count drops to 0 → Stopped.
#[test]
fn test_graceful_shutdown_drain_completes() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    signal.trigger_shutdown(ShutdownMode::Graceful);
    assert_eq!(handle.drain_status().state, ShutdownState::Draining);

    // Simulate in-flight work finishing.
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 1);
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);

    // Auto-complete drain.
    signal.complete_drain();
    assert_eq!(handle.drain_status().state, ShutdownState::Stopped);
    assert!(
        !handle.is_shutting_down(),
        "Stopped is not a shutting-down state"
    );
}

/// Full lifecycle: Running → Draining → Stopped with busy operations.
#[test]
fn test_full_lifecycle_with_busy_operations() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    // Phase 1: Running
    assert_eq!(handle.drain_status().state, ShutdownState::Running);
    assert_eq!(handle.busy_count(), 0);

    // Simulate some work starting.
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 2);

    // Phase 2: Shutdown triggered while work is in progress.
    signal.trigger_shutdown(ShutdownMode::Graceful);
    assert!(handle.is_shutting_down());
    assert_eq!(handle.drain_status().state, ShutdownState::Draining);
    assert_eq!(handle.busy_count(), 2);

    // Phase 3: Work completes one by one.
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 1);
    assert_eq!(handle.drain_status().state, ShutdownState::Draining);

    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
    signal.complete_drain();

    // Phase 4: Stopped.
    assert_eq!(handle.drain_status().state, ShutdownState::Stopped);
    assert!(!handle.is_shutting_down());
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Error path: idempotency of repeated shutdown triggers
// ═════════════════════════════════════════════════════════════════════════════

/// Triggering shutdown multiple times should be idempotent.
#[test]
fn test_repeated_shutdown_triggers_are_idempotent() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    signal.trigger_shutdown(ShutdownMode::Graceful);
    let state_after_first = handle.drain_status().state;

    signal.trigger_shutdown(ShutdownMode::Graceful);
    let state_after_second = handle.drain_status().state;

    assert_eq!(
        state_after_first, state_after_second,
        "repeated shutdown should not change state"
    );
    assert_eq!(handle.busy_count(), 0);
}

/// Double decrement below 0 must not panic (saturation at 0).
#[test]
fn test_decrement_busy_does_not_underflow() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    // Decrement without prior increment — busy_count should saturate at 0.
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0, "busy_count must not underflow");
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Boundary value: DrainStatus snapshot accuracy
// ═════════════════════════════════════════════════════════════════════════════

/// DrainStatus snapshot reflects exact busy_count at call time.
#[test]
fn test_drain_status_snapshot_accuracy() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    signal.trigger_shutdown(ShutdownMode::Graceful);

    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();

    let snapshot = handle.drain_status();
    assert_eq!(snapshot.busy_count, 3);
    assert_eq!(snapshot.state, ShutdownState::Draining);
    assert!(snapshot.is_draining);

    handle.decrement_busy();
    let snapshot2 = handle.drain_status();
    assert_eq!(snapshot2.busy_count, 2);
}

/// DrainStatus snapshot when not shutting down.
#[test]
fn test_drain_status_snapshot_running() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    let snapshot = handle.drain_status();
    assert_eq!(snapshot.state, ShutdownState::Running);
    assert_eq!(snapshot.busy_count, 0);
    assert!(!snapshot.is_draining);
}

/// DrainStatus snapshot after drain completes (Stopped).
#[test]
fn test_drain_status_snapshot_stopped() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    signal.trigger_shutdown(ShutdownMode::Graceful);
    signal.complete_drain();

    let snapshot = handle.drain_status();
    assert_eq!(snapshot.state, ShutdownState::Stopped);
    assert_eq!(snapshot.busy_count, 0);
    assert!(!snapshot.is_draining);
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. State transition: ShutdownMode behavioral impact
// ═════════════════════════════════════════════════════════════════════════════

/// Graceful shutdown: drains in-flight work before completing.
#[test]
fn test_graceful_mode_waits_for_drain() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    handle.increment_busy();
    signal.trigger_shutdown(ShutdownMode::Graceful);

    assert!(handle.is_shutting_down());
    assert_eq!(handle.drain_status().state, ShutdownState::Draining);
    assert_eq!(handle.busy_count(), 1);

    // Drain not complete while work is in progress.
    signal.complete_drain();
    assert_eq!(
        handle.drain_status().state,
        ShutdownState::Draining,
        "should remain Draining while busy_count > 0"
    );

    // Complete the work.
    handle.decrement_busy();
    signal.complete_drain();
    assert_eq!(handle.drain_status().state, ShutdownState::Stopped);
}

/// Immediate shutdown: bypasses drain, goes straight to forceful state.
#[test]
fn test_immediate_mode_skips_drain() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    handle.increment_busy();
    handle.increment_busy();

    signal.trigger_shutdown(ShutdownMode::Forceful);

    assert!(handle.is_shutting_down());
    assert!(handle.is_forceful());
    assert_eq!(
        handle.drain_status().state,
        ShutdownState::ForcefulShuttingDown
    );
    // Busy count is irrelevant in forceful mode — state does not depend on it.
    assert_eq!(handle.busy_count(), 2);
}

/// Forceful escalation from Graceful state.
#[test]
fn test_escalation_from_graceful_to_forceful() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    handle.increment_busy();
    signal.trigger_shutdown(ShutdownMode::Graceful);

    assert_eq!(handle.drain_status().state, ShutdownState::Draining);
    assert!(!handle.is_forceful());

    // Escalate to forceful.
    let escalated = handle.escalate_to_forceful();
    assert!(escalated, "escalation should succeed from Draining state");
    assert!(handle.is_forceful());
    assert_eq!(
        handle.drain_status().state,
        ShutdownState::ForcefulShuttingDown
    );
}

/// Escalation from Running state should fail (not yet shutting down).
#[test]
fn test_escalation_from_running_fails() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    let escalated = handle.escalate_to_forceful();
    assert!(!escalated, "escalation should fail from Running state");
    assert!(!handle.is_forceful());
}

/// Escalation when already forceful should fail (already escalated).
#[test]
fn test_escalation_already_forceful_fails() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    signal.trigger_shutdown(ShutdownMode::Forceful);
    assert!(handle.is_forceful());

    let escalated = handle.escalate_to_forceful();
    assert!(
        !escalated,
        "escalation should fail when already ForcefulShuttingDown"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Clone and Debug behavior
// ═════════════════════════════════════════════════════════════════════════════

/// Cloned handle shares the same underlying signal.
#[test]
fn test_clone_shares_signal() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());
    let cloned = handle.clone();

    signal.trigger_shutdown(ShutdownMode::Graceful);

    assert!(
        cloned.is_shutting_down(),
        "cloned handle should reflect signal state"
    );
    assert_eq!(cloned.drain_status().state, ShutdownState::Draining);
}

/// Debug output includes busy_count.
#[test]
fn test_debug_output_includes_busy_count() {
    let signal = Arc::new(MockTransitionSignal::new());
    let handle = ShutdownHandle::new(signal.clone());

    handle.increment_busy();
    let debug = format!("{:?}", handle);
    assert!(
        debug.contains("busy_count"),
        "Debug output should include busy_count field"
    );
}
