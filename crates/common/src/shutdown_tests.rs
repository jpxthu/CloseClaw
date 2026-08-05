//! Unit tests for ShutdownSignal trait and DrainStatus.
//!
//! Covers Step 1.4 (Gap 3): verifies trait default implementations
//! and DrainStatus field structure.

use super::*;

/// Minimal ShutdownSignal impl that uses only default methods for
/// increment_busy_tracked / decrement_busy_tracked.
struct MinimalSignal {
    busy: std::sync::atomic::AtomicUsize,
}

impl MinimalSignal {
    fn new() -> Self {
        Self {
            busy: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl ShutdownSignal for MinimalSignal {
    fn is_shutting_down(&self) -> bool {
        false
    }

    fn increment_busy(&self) {
        self.busy.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn decrement_busy(&self) {
        self.busy.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn busy_count(&self) -> usize {
        self.busy.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn escalate_to_forceful(&self) -> bool {
        false
    }

    fn is_forceful(&self) -> bool {
        false
    }

    fn drain_status(&self) -> DrainStatus {
        DrainStatus {
            state: ShutdownState::Running,
            busy_count: self.busy_count(),
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
    // Default implementations for increment_busy_tracked / decrement_busy_tracked
}

#[test]
fn test_default_increment_busy_tracked_delegates_to_increment_busy() {
    let signal = MinimalSignal::new();
    assert_eq!(signal.busy_count(), 0);

    // Default impl should call increment_busy and return 0
    let id = signal.increment_busy_tracked("test");
    assert_eq!(id, 0, "default impl returns 0 (untracked)");
    assert_eq!(signal.busy_count(), 1, "busy_count should be incremented");
}

#[test]
fn test_default_decrement_busy_tracked_delegates_to_decrement_busy() {
    let signal = MinimalSignal::new();
    signal.increment_busy();
    signal.increment_busy();
    assert_eq!(signal.busy_count(), 2);

    // Default impl should call decrement_busy
    signal.decrement_busy_tracked(99); // id is ignored by default
    assert_eq!(signal.busy_count(), 1, "busy_count should be decremented");
}

#[test]
fn test_default_tracked_methods_do_not_panic() {
    let signal = MinimalSignal::new();
    // Multiple calls should not panic
    let id = signal.increment_busy_tracked("op-1");
    signal.decrement_busy_tracked(id);
    let id = signal.increment_busy_tracked("op-2");
    signal.decrement_busy_tracked(id);
}

#[test]
fn test_drain_status_pending_items_is_vec_string() {
    // Verify the type of pending_items
    let status = DrainStatus {
        state: ShutdownState::Running,
        busy_count: 0,
        is_draining: false,
        pending_items: vec!["test".to_owned()],
    };
    assert_eq!(status.pending_items.len(), 1);
    assert_eq!(status.pending_items[0], "test");
}

#[test]
fn test_drain_status_clone_preserves_pending_items() {
    let status = DrainStatus {
        state: ShutdownState::Draining,
        busy_count: 3,
        is_draining: true,
        pending_items: vec!["op-1".to_owned(), "op-2".to_owned()],
    };
    let cloned = status.clone();
    assert_eq!(cloned.pending_items, status.pending_items);
    assert_eq!(cloned.busy_count, 3);
    assert_eq!(cloned.is_draining, true);
}

#[test]
fn test_drain_status_debug_includes_pending_items() {
    let status = DrainStatus {
        state: ShutdownState::Running,
        busy_count: 0,
        is_draining: false,
        pending_items: vec!["gateway".to_owned()],
    };
    let debug = format!("{:?}", status);
    assert!(debug.contains("pending_items"));
    assert!(debug.contains("gateway"));
}
