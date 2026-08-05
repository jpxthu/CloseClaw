//! Unit tests for the shutdown coordinator and handle.
//!
//! Extracted from `shutdown.rs` to keep source files within the
//! 1000-line CONTRIBUTING.md limit.

use crate::shutdown::*;
use closeclaw_common::ShutdownSignal;
use std::time::Duration;

#[test]
fn test_shutdown_state_from_u8() {
    assert_eq!(ShutdownState::from_u8(0), ShutdownState::Running);
    assert_eq!(ShutdownState::from_u8(1), ShutdownState::ShuttingDown);
    assert_eq!(ShutdownState::from_u8(2), ShutdownState::Draining);
    assert_eq!(ShutdownState::from_u8(3), ShutdownState::Stopped);
    assert_eq!(
        ShutdownState::from_u8(4),
        ShutdownState::ForcefulShuttingDown
    );
    // Invalid values default to Running
    assert_eq!(ShutdownState::from_u8(99), ShutdownState::Running);
}

#[test]
fn test_coordinator_initial_state() {
    let coordinator = ShutdownCoordinator::new();
    assert_eq!(coordinator.state(), ShutdownState::Running);
}

#[test]
fn test_coordinator_try_start_shutdown() {
    let coordinator = ShutdownCoordinator::new();

    // First call succeeds
    assert!(coordinator.try_start_shutdown());
    assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);

    // Second call fails (already shutting down)
    assert!(!coordinator.try_start_shutdown());
    assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);
}

#[test]
fn test_coordinator_state_transitions() {
    let coordinator = ShutdownCoordinator::new();

    coordinator.try_start_shutdown();
    assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);

    coordinator.start_drain();
    assert_eq!(coordinator.state(), ShutdownState::Draining);

    coordinator.mark_stopped();
    assert_eq!(coordinator.state(), ShutdownState::Stopped);
}

#[test]
fn test_coordinator_escalate_to_forceful_success() {
    let coordinator = ShutdownCoordinator::new();
    coordinator.try_start_shutdown();
    assert!(coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_coordinator_escalate_to_forceful_fails_when_running() {
    let coordinator = ShutdownCoordinator::new();
    assert!(!coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::Running);
}

#[test]
fn test_coordinator_escalate_to_forceful_succeeds_when_stopped() {
    let coordinator = ShutdownCoordinator::new();
    coordinator.try_start_shutdown();
    coordinator.start_drain();
    coordinator.mark_stopped();
    assert!(coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_coordinator_escalate_to_forceful_fails_when_already_forceful() {
    let coordinator = ShutdownCoordinator::new();
    coordinator.try_start_shutdown();
    assert!(coordinator.escalate_to_forceful());
    // Second escalate should fail (already forceful, not ShuttingDown)
    assert!(!coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_coordinator_mode() {
    let coordinator = ShutdownCoordinator::new();
    assert_eq!(coordinator.mode(), ShutdownMode::Graceful);

    coordinator.try_start_shutdown();
    assert_eq!(coordinator.mode(), ShutdownMode::Graceful);

    coordinator.escalate_to_forceful();
    assert_eq!(coordinator.mode(), ShutdownMode::Forceful);
}

#[test]
fn test_shutdown_handle_initial_state() {
    let handle = ShutdownHandle::new();
    assert_eq!(handle.state(), ShutdownState::Running);
    assert!(!handle.is_shutting_down());
    assert!(!handle.is_stopped());
    assert!(!handle.is_forceful());
    assert_eq!(handle.mode(), ShutdownMode::Graceful);
}

#[test]
fn test_shutdown_handle_escalate_success() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    assert!(handle.escalate_to_forceful());
    assert!(handle.is_forceful());
    assert_eq!(handle.mode(), ShutdownMode::Forceful);
}

#[test]
fn test_shutdown_handle_escalate_fails_when_running() {
    let handle = ShutdownHandle::new();
    assert!(!handle.escalate_to_forceful());
    assert!(!handle.is_forceful());
}

#[test]
fn test_shutdown_handle_is_shutting_down_in_forceful() {
    let handle = ShutdownHandle::new();
    handle.try_start_shutdown();
    handle.escalate_to_forceful();
    assert!(handle.is_shutting_down());
}

#[test]
fn test_shutdown_handle_subscribe_drain() {
    let handle = ShutdownHandle::new();
    let mut rx = handle.subscribe_drain();
    // No message yet
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_initiate_shutdown_first_caller_wins() {
    let handle = ShutdownHandle::new();
    // Register a busy operation so drain doesn't complete immediately
    handle.increment_busy();

    // Phase 0: set gate immediately (simulates signal reception)
    handle.try_start_shutdown();
    assert!(
        handle.is_shutting_down(),
        "gate should be active after Phase 0"
    );

    // First initiate succeeds (gate already set by Phase 0)
    let handle2 = handle.clone();
    tokio::spawn(async move {
        handle2.initiate_shutdown().await;
    });

    // Give it a moment to enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(handle.is_shutting_down());

    // Release the busy count so drain can complete
    handle.decrement_busy();
}

#[test]
fn test_shutdown_state_debug() {
    assert_eq!(format!("{:?}", ShutdownState::Running), "Running");
    assert_eq!(format!("{:?}", ShutdownState::ShuttingDown), "ShuttingDown");
    assert_eq!(format!("{:?}", ShutdownState::Draining), "Draining");
    assert_eq!(format!("{:?}", ShutdownState::Stopped), "Stopped");
    assert_eq!(
        format!("{:?}", ShutdownState::ForcefulShuttingDown),
        "ForcefulShuttingDown"
    );
}

#[test]
fn test_shutdown_mode_debug() {
    assert_eq!(format!("{:?}", ShutdownMode::Graceful), "Graceful");
    assert_eq!(format!("{:?}", ShutdownMode::Forceful), "Forceful");
}

#[test]
fn test_drain_poll_interval_test_mode() {
    // In test mode, drain_poll_interval should return 100ms (not 2s)
    assert_eq!(drain_poll_interval(), std::time::Duration::from_millis(100));
}

// ── Step 1.3: try_start_forceful_shutdown unit tests ──────────────

#[test]
fn test_try_start_forceful_shutdown_success() {
    let coordinator = ShutdownCoordinator::new();
    assert_eq!(coordinator.state(), ShutdownState::Running);

    assert!(coordinator.try_start_forceful_shutdown());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_try_start_forceful_shutdown_fails_when_not_running() {
    let coordinator = ShutdownCoordinator::new();
    // Transition to ShuttingDown first
    coordinator.try_start_shutdown();
    assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);

    // Should fail — not in Running state
    assert!(!coordinator.try_start_forceful_shutdown());
    // State unchanged
    assert_eq!(coordinator.state(), ShutdownState::ShuttingDown);
}

#[test]
fn test_try_start_forceful_shutdown_fails_when_already_forceful() {
    let coordinator = ShutdownCoordinator::new();
    // First call succeeds
    assert!(coordinator.try_start_forceful_shutdown());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);

    // Second call fails — already ForcefulShuttingDown
    assert!(!coordinator.try_start_forceful_shutdown());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_busy_count_unchanged_in_forceful_mode() {
    let handle = ShutdownHandle::new();

    // Start a graceful shutdown with pending work
    handle.try_start_shutdown();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 1);

    // Escalate to forceful
    assert!(handle.escalate_to_forceful());
    assert!(handle.is_forceful());

    // busy_count is still 1 — forceful mode doesn't clear it;
    // the drain path simply skips waiting for it to reach 0
    assert_eq!(handle.busy_count(), 1);

    // Decrement still works normally
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
}

#[tokio::test]
async fn test_subscribe_drain_triggers_on_escalation() {
    let handle = ShutdownHandle::new();
    let mut rx = handle.subscribe_drain();
    handle.increment_busy();

    // Spawn initiate_shutdown — it will block on drain because busy_count > 0
    let h = handle.clone();
    tokio::spawn(async move {
        h.initiate_shutdown().await;
    });
    // Let it enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(handle.is_shutting_down());

    // Escalate — drain_done_tx fires during initiate_shutdown,
    // so the subscriber should receive the signal
    handle.escalate_to_forceful();
    // Release busy count so drain can finalize
    handle.decrement_busy();

    // Wait for shutdown to finish
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // The subscriber received at least one drain signal
    // (sent in initiate_shutdown before the drain loop)
    assert!(rx.try_recv().is_ok());
}

#[test]
fn test_handle_escalate_idempotent() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();

    // First escalation succeeds
    assert!(handle.escalate_to_forceful());
    assert!(handle.is_forceful());
    assert_eq!(handle.mode(), ShutdownMode::Forceful);

    // Second escalation is a no-op (already forceful)
    assert!(!handle.escalate_to_forceful());
    assert!(handle.is_forceful());
}

#[test]
fn test_is_shutting_down_true_when_draining() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    handle.set_draining_for_test();

    // Draining is still "shutting down" — components should reject new work
    assert!(handle.is_shutting_down());
    assert!(!handle.is_forceful());
}

#[test]
fn test_is_shutting_down_false_when_stopped() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    handle.set_draining_for_test();
    handle.set_stopped_for_test();

    // Stopped is not "shutting down" — the shutdown is complete
    assert!(!handle.is_shutting_down());
    assert!(!handle.is_forceful());
}

#[tokio::test]
async fn test_graceful_drain_timeout() {
    // After timeout, drain completes even if busy_count > 0.
    let handle = ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_millis(100));
    // Register two pending operations — neither will complete
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 2);

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Wait for timeout to fire + buffer
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Shutdown should have completed despite busy_count > 0
    shutdown_handle.await.unwrap();
    assert!(handle.is_stopped(), "drain should complete after timeout");
    // busy_count was not cleared by the drain
    assert_eq!(handle.busy_count(), 2);
}

#[tokio::test]
async fn test_drain_timeout_returns_remaining_count() {
    // Timeout leaves busy_count intact — caller gets the remaining count.
    let handle = ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_millis(200));
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 3);

    let h = handle.clone();
    tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Wait for timeout + buffer
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert!(handle.is_stopped());
    // busy_count still reflects the 3 in-flight operations
    assert_eq!(handle.busy_count(), 3);
}

#[tokio::test]
async fn test_drain_completes_on_zero_count() {
    // When busy_count reaches 0, drain completes immediately
    // without waiting for the full timeout.
    let handle = ShutdownHandle::new().with_drain_timeout(std::time::Duration::from_secs(10));
    handle.increment_busy();
    handle.increment_busy();

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Let it enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(!handle.is_stopped());

    // Complete both operations
    handle.decrement_busy();
    handle.decrement_busy();

    // Should complete quickly, not wait for 10s timeout
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_handle).await;
    assert!(
        result.is_ok(),
        "drain should complete when busy_count hits 0"
    );
    assert!(handle.is_stopped());
    assert_eq!(handle.busy_count(), 0);
}

#[tokio::test]
async fn test_forceful_skips_drain() {
    // Forceful mode terminates immediately, ignoring busy_count.
    let handle = ShutdownHandle::new();
    for _ in 0..50 {
        handle.increment_busy();
    }

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(handle.is_shutting_down());
    assert!(!handle.is_stopped());

    // Escalate to forceful — should terminate immediately
    handle.escalate_to_forceful();
    shutdown_handle.await.unwrap();

    assert!(handle.is_stopped());
    // busy_count unchanged — forceful skips drain
    assert_eq!(handle.busy_count(), 50);
}

// ── Step 1.4: drain_status & Phase 0 gate tests ────────────────

#[test]
fn test_drain_status_running() {
    let handle = ShutdownHandle::new();
    let status = handle.drain_status();
    assert_eq!(status.state, ShutdownState::Running);
    assert_eq!(status.busy_count, 0);
    assert!(!status.is_draining);
}

#[test]
fn test_drain_status_shutting_down() {
    let handle = ShutdownHandle::new();
    handle.try_start_shutdown();
    let status = handle.drain_status();
    assert_eq!(status.state, ShutdownState::ShuttingDown);
    assert_eq!(status.busy_count, 0);
    assert!(!status.is_draining);
}

#[test]
fn test_drain_status_with_busy_count() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();
    let status = handle.drain_status();
    assert_eq!(status.state, ShutdownState::Running);
    assert_eq!(status.busy_count, 3);
    assert!(!status.is_draining);
}

#[test]
fn test_phase0_gate_set_on_signal() {
    // Verify that try_start_shutdown() sets the gate to ShuttingDown
    // synchronously — the gate must be active before any async drain
    // logic begins (simulating Phase 0 in tokio::select!).
    let handle = ShutdownHandle::new();
    assert_eq!(handle.state(), ShutdownState::Running);
    assert!(!handle.is_shutting_down());

    // Simulate signal arrival: gate must flip immediately
    handle.try_start_shutdown();
    assert_eq!(handle.state(), ShutdownState::ShuttingDown);
    assert!(handle.is_shutting_down());
    assert_eq!(handle.mode(), ShutdownMode::Graceful);

    // Verify drain_status reflects the gate being set
    let status = handle.drain_status();
    assert_eq!(status.state, ShutdownState::ShuttingDown);
}

// ── Step 1.3: escalate_to_forceful from Draining/Stopped ──────────

#[test]
fn test_escalate_from_draining() {
    let coordinator = ShutdownCoordinator::new();
    coordinator.try_start_shutdown();
    coordinator.start_drain();
    assert_eq!(coordinator.state(), ShutdownState::Draining);

    assert!(coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

#[test]
fn test_escalate_from_stopped() {
    let coordinator = ShutdownCoordinator::new();
    coordinator.try_start_shutdown();
    coordinator.start_drain();
    coordinator.mark_stopped();
    assert_eq!(coordinator.state(), ShutdownState::Stopped);

    assert!(coordinator.escalate_to_forceful());
    assert_eq!(coordinator.state(), ShutdownState::ForcefulShuttingDown);
}

// ── Step 1.4: Gap 2 — initiate_shutdown return value tests ─────

#[tokio::test]
async fn test_initiate_shutdown_returns_zero_when_drained() {
    // When busy_count reaches 0, initiate_shutdown returns 0.
    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_secs(10));
    handle.increment_busy();

    // Spawn a task that decrements busy_count after a short delay,
    // allowing the drain to complete.
    let h_dec = handle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        h_dec.decrement_busy();
    });

    let h = handle.clone();
    let result = tokio::spawn(async move { h.initiate_shutdown().await })
        .await
        .unwrap();

    assert_eq!(result, 0, "should return 0 when all ops drained");
    assert!(handle.is_stopped());
}

#[tokio::test]
async fn test_initiate_shutdown_returns_remaining_on_timeout() {
    // When drain times out, initiate_shutdown returns the remaining
    // busy_count.
    let handle = ShutdownHandle::new().with_drain_timeout(Duration::from_millis(50));
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();

    let h = handle.clone();
    let result = tokio::spawn(async move { h.initiate_shutdown().await })
        .await
        .unwrap();

    assert_eq!(result, 3, "should return remaining busy_count on timeout");
    assert!(handle.is_stopped());
}

#[tokio::test]
async fn test_initiate_shutdown_forceful_returns_zero() {
    // Forceful mode skips drain and returns 0.
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();

    // Start graceful, then escalate to forceful
    handle.try_start_shutdown();
    handle.escalate_to_forceful();

    let h = handle.clone();
    let result = tokio::spawn(async move { h.initiate_shutdown().await })
        .await
        .unwrap();

    assert_eq!(result, 0, "forceful mode should return 0");
    assert!(handle.is_stopped());
    // busy_count unchanged — forceful skips drain
    assert_eq!(handle.busy_count(), 3);
}

// ── Step 1.4: Gap 3 — ShutdownSignal trait default impl tests ───

struct UntrackedSignal;

impl closeclaw_common::ShutdownSignal for UntrackedSignal {
    fn is_shutting_down(&self) -> bool {
        false
    }
    fn increment_busy(&self) {}
    fn decrement_busy(&self) {}
    fn busy_count(&self) -> usize {
        0
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
            busy_count: 0,
            is_draining: false,
            pending_items: Vec::new(),
        }
    }
    // Uses default implementations for increment_busy_tracked /
    // decrement_busy_tracked — no override needed.
}

#[test]
fn test_default_increment_busy_tracked_returns_zero() {
    let signal = UntrackedSignal;
    // Default impl calls increment_busy() and returns 0
    let id = signal.increment_busy_tracked("test op");
    assert_eq!(id, 0, "default impl should return 0 (untracked)");
}

#[test]
fn test_default_decrement_busy_tracked_no_panic() {
    let signal = UntrackedSignal;
    // Default impl calls decrement_busy() — should not panic
    signal.decrement_busy_tracked(42);
}

#[test]
fn test_daemon_handle_tracked_returns_unique_ids() {
    let handle = ShutdownHandle::new();
    let id1 = handle.increment_busy_tracked("op-1");
    let id2 = handle.increment_busy_tracked("op-2");
    let id3 = handle.increment_busy_tracked("op-3");

    assert_ne!(id1, id2, "tracked ids should be unique");
    assert_ne!(id2, id3, "tracked ids should be unique");
    assert_ne!(id1, id3, "tracked ids should be unique");

    // All three increments should be reflected in busy_count
    assert_eq!(handle.busy_count(), 3);
}

#[test]
fn test_daemon_handle_tracked_decrement_removes_correct_entry() {
    let handle = ShutdownHandle::new();
    let id1 = handle.increment_busy_tracked("op-1");
    let _id2 = handle.increment_busy_tracked("op-2");

    assert_eq!(handle.busy_count(), 2);

    // Remove op-1 — op-2 should remain
    handle.decrement_busy_tracked(id1);
    assert_eq!(handle.busy_count(), 1);

    let status = handle.drain_status();
    assert_eq!(status.pending_items.len(), 1);
    assert!(status.pending_items.contains(&"op-2".to_owned()));
}

#[test]
fn test_daemon_handle_drain_status_pending_items_snapshot() {
    let handle = ShutdownHandle::new();
    let id1 = handle.increment_busy_tracked("gateway-msg");
    let _id2 = handle.increment_busy_tracked("tool-exec");

    let status = handle.drain_status();
    assert_eq!(status.pending_items.len(), 2);
    assert!(status.pending_items.contains(&"gateway-msg".to_owned()));
    assert!(status.pending_items.contains(&"tool-exec".to_owned()));

    // Remove one — snapshot should reflect the change
    handle.decrement_busy_tracked(id1);
    let status2 = handle.drain_status();
    assert_eq!(status2.pending_items.len(), 1);
    assert!(status2.pending_items.contains(&"tool-exec".to_owned()));
}

#[test]
fn test_daemon_handle_drain_status_empty_when_no_tracked() {
    let handle = ShutdownHandle::new();
    let status = handle.drain_status();
    assert!(status.pending_items.is_empty());
}

#[test]
fn test_daemon_handle_untracked_increment_decrement_works() {
    let handle = ShutdownHandle::new();
    // Untracked methods should not affect pending_items
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 2);
    let status = handle.drain_status();
    assert!(status.pending_items.is_empty());

    handle.decrement_busy();
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
}
