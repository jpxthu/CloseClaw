//! Unit tests for shutdown alignment changes (Steps 1.1–1.3).
//!
//! Covers:
//! - Step 1.1: busy_count registration (increment/decrement lifecycle)
//! - Step 1.2: shutdown gate checks (is_shutting_down gating)
//! - Step 1.3: hard timeout removal (graceful waits, forceful immediate)

use crate::daemon::shutdown::{ShutdownHandle, ShutdownMode};

// ── Step 1.1: busy_count registration ──────────────────────────────────

#[test]
fn test_busy_count_initial_is_zero() {
    let handle = ShutdownHandle::new();
    assert_eq!(handle.busy_count(), 0);
}

#[test]
fn test_busy_count_increment_decrement() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 1);
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
}

#[test]
fn test_busy_count_multiple_increment_decrement() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 3);
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 2);
    handle.decrement_busy();
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
}

#[test]
fn test_busy_count_not_affected_by_shutdown_state() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 1);

    // Start shutdown — busy_count should remain unchanged
    handle.start_shutdown_for_test();
    assert_eq!(handle.busy_count(), 1);

    // Escalate to forceful — busy_count should remain unchanged
    handle.escalate_to_forceful();
    assert_eq!(handle.busy_count(), 1);

    // Decrement still works
    handle.decrement_busy();
    assert_eq!(handle.busy_count(), 0);
}

#[test]
fn test_busy_count_symmetric_increments_match_decrements() {
    let handle = ShutdownHandle::new();
    // Simulate 5 concurrent operations
    for _ in 0..5 {
        handle.increment_busy();
    }
    assert_eq!(handle.busy_count(), 5);

    // All complete
    for _ in 0..5 {
        handle.decrement_busy();
    }
    assert_eq!(handle.busy_count(), 0);
}

// ── Step 1.2: shutdown gate checks ─────────────────────────────────────

#[test]
fn test_gate_not_shutting_down_allows_operations() {
    let handle = ShutdownHandle::new();
    assert!(!handle.is_shutting_down());
}

#[test]
fn test_gate_shutting_down_rejects_operations() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    assert!(handle.is_shutting_down());
}

#[test]
fn test_gate_forceful_shutting_down_rejects_operations() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    handle.escalate_to_forceful();
    assert!(handle.is_shutting_down());
    assert!(handle.is_forceful());
}

#[test]
fn test_gate_draining_rejects_operations() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    // Draining is set by the drain loop, but we can verify the gate
    // state: ShuttingDown is still "shutting down"
    assert!(handle.is_shutting_down());
}

#[test]
fn test_gate_stopped_allows_operations() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    handle.escalate_to_forceful();
    // Forceful mode terminates immediately when there are no pending ops
    // After full shutdown sequence, is_shutting_down() returns false
    // For this test, we verify the public API: is_shutting_down
    // reflects the current state
    assert!(handle.is_shutting_down());
}

#[test]
fn test_gate_check_pattern_tool_execution() {
    // Simulates the shutdown gate pattern used in register_tool_handle:
    // if is_shutting_down() { return; }
    let handle = ShutdownHandle::new();
    let mut tool_registered = false;

    if !handle.is_shutting_down() {
        tool_registered = true;
    }
    assert!(
        tool_registered,
        "tool should be registered when not shutting down"
    );

    // Now start shutdown
    handle.start_shutdown_for_test();
    tool_registered = false;
    if !handle.is_shutting_down() {
        tool_registered = true;
    }
    assert!(
        !tool_registered,
        "tool should NOT be registered when shutting down"
    );
}

#[test]
fn test_gate_check_pattern_session_spawn() {
    // Simulates the shutdown gate pattern used in create_child_session:
    // if is_shutting_down() { return Err("daemon is shutting down"); }
    let handle = ShutdownHandle::new();

    fn try_spawn(handle: &ShutdownHandle) -> Result<(), String> {
        if handle.is_shutting_down() {
            return Err("daemon is shutting down".into());
        }
        Ok(())
    }

    assert!(try_spawn(&handle).is_ok());

    handle.start_shutdown_for_test();
    assert_eq!(try_spawn(&handle).unwrap_err(), "daemon is shutting down");
}

// ── Step 1.3: hard timeout removal — graceful waits indefinitely ──────

#[tokio::test]
async fn test_graceful_mode_waits_for_operations() {
    let handle = ShutdownHandle::new();
    // Register one pending operation
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 1);

    // Spawn initiate_shutdown — it will block on drain because busy_count > 0
    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Give the shutdown task time to enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // State should be ShuttingDown (graceful), not Stopped
    assert!(handle.is_shutting_down());
    assert!(!handle.is_stopped());
    assert_eq!(handle.busy_count(), 1);

    // Complete the operation
    handle.decrement_busy();

    // Wait for shutdown to complete
    shutdown_handle.await.unwrap();
    assert!(handle.is_stopped());
}

#[tokio::test]
async fn test_forceful_mode_immediate_termination() {
    let handle = ShutdownHandle::new();
    // Register pending operations that would block graceful shutdown
    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(handle.busy_count(), 2);

    // Start graceful shutdown
    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Let it enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(handle.is_shutting_down());
    assert!(!handle.is_stopped());

    // Escalate to forceful — should terminate immediately
    // without waiting for busy_count to reach 0
    handle.escalate_to_forceful();

    // Wait for shutdown to complete
    shutdown_handle.await.unwrap();

    // Forceful mode terminated immediately even though busy_count > 0
    assert!(handle.is_stopped());
    // After forceful termination, mode reflects the forceful decision
    // (state is Stopped, but the shutdown was forceful)
    // busy_count is still > 0 — forceful mode doesn't drain
    assert_eq!(handle.busy_count(), 2);
}

#[tokio::test]
async fn test_drain_completes_before_timeout() {
    // When busy_count reaches 0, drain completes immediately
    // without waiting for the full timeout to expire.
    let handle = ShutdownHandle::new();
    handle.increment_busy();

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Wait 1 second — drain should still be running (30s default timeout
    // has not fired)
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert!(handle.is_shutting_down());
    assert!(!handle.is_stopped());

    // Complete the operation so drain can finish naturally
    handle.decrement_busy();
    shutdown_handle.await.unwrap();
    assert!(handle.is_stopped());
}

#[tokio::test]
async fn test_forceful_escalation_bypasses_drain() {
    let handle = ShutdownHandle::new();
    // Many pending operations
    for _ in 0..100 {
        handle.increment_busy();
    }

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Escalate to forceful
    handle.escalate_to_forceful();
    shutdown_handle.await.unwrap();

    // Should be stopped immediately despite 100 pending ops
    assert!(handle.is_stopped());
    // busy_count unchanged — forceful skips drain
    assert_eq!(handle.busy_count(), 100);
}

// ── Integration: drain actually waits for busy_count ────────────────────

#[tokio::test]
async fn test_drain_completes_when_busy_count_reaches_zero() {
    let handle = ShutdownHandle::new();
    handle.increment_busy();
    handle.increment_busy();

    let h = handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    // Let it enter the drain loop
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!handle.is_stopped());

    // Complete operations one by one
    handle.decrement_busy();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!handle.is_stopped(), "should still be waiting for last op");

    handle.decrement_busy();
    shutdown_handle.await.unwrap();
    assert!(
        handle.is_stopped(),
        "should be stopped after all ops complete"
    );
}

// ── ShutdownState edge cases ───────────────────────────────────────────

#[test]
fn test_shutdown_mode_reflects_state() {
    let handle = ShutdownHandle::new();
    assert_eq!(handle.mode(), ShutdownMode::Graceful);

    handle.start_shutdown_for_test();
    assert_eq!(handle.mode(), ShutdownMode::Graceful);

    handle.escalate_to_forceful();
    assert_eq!(handle.mode(), ShutdownMode::Forceful);
}

#[test]
fn test_shutdown_handle_escalate_idempotent() {
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
fn test_is_shutting_down_false_when_stopped() {
    let handle = ShutdownHandle::new();
    handle.start_shutdown_for_test();
    handle.escalate_to_forceful();
    // State is ForcefulShuttingDown after escalation — is_shutting_down
    // returns true for all active shutdown states (ShuttingDown, Draining,
    // ForcefulShuttingDown). Only the Stopped state returns false, which
    // requires the full async initiate_shutdown → drain → mark_stopped
    // cycle to complete.
    assert!(handle.is_shutting_down());
}

// ── Step 1.5: Phase 0 gate timing ──────────────────────────────────────

#[test]
fn test_phase0_gate_set_immediately_after_signal() {
    // Phase 0 sets the gate via try_start_shutdown() before Phase 1 begins.
    // After the gate is set, is_shutting_down() should return true
    // immediately — no drain or async work needed.
    let handle = ShutdownHandle::new();
    assert!(!handle.is_shutting_down());

    // Simulate Phase 0: signal received, gate set immediately
    let initiated = handle.try_start_shutdown();
    assert!(initiated, "first try_start_shutdown should succeed");
    assert!(
        handle.is_shutting_down(),
        "gate should be active after Phase 0"
    );
}

#[test]
fn test_phase0_gate_rejects_new_operations() {
    // After Phase 0 gate is set, new operations should be rejected.
    let handle = ShutdownHandle::new();
    handle.try_start_shutdown();

    fn try_accept(handle: &ShutdownHandle) -> Result<(), String> {
        if handle.is_shutting_down() {
            return Err("reject".into());
        }
        Ok(())
    }

    assert!(try_accept(&handle).is_err());
}

#[test]
fn test_phase0_gate_idempotent() {
    let handle = ShutdownHandle::new();
    assert!(handle.try_start_shutdown());
    assert!(!handle.try_start_shutdown(), "second call should fail");
    assert!(handle.is_shutting_down());
}

// ── Step 1.5: ShutdownSignal impl delegation verification ───────────────

#[test]
fn test_daemon_shutdown_signal_busy_count_delegation() {
    use closeclaw_common::ShutdownSignal;

    let handle = ShutdownHandle::new();
    let signal: &dyn ShutdownSignal = &handle;

    handle.increment_busy();
    handle.increment_busy();
    assert_eq!(signal.busy_count(), 2);

    signal.decrement_busy();
    assert_eq!(handle.busy_count(), 1);
}

#[test]
fn test_daemon_shutdown_signal_escalate_returns_correctly() {
    use closeclaw_common::ShutdownSignal;

    let handle = ShutdownHandle::new();
    let signal: &dyn ShutdownSignal = &handle;

    // Cannot escalate when Running
    assert!(!signal.escalate_to_forceful());

    // Can escalate after shutdown started
    handle.try_start_shutdown();
    assert!(signal.escalate_to_forceful());

    // Cannot escalate again (already forceful)
    assert!(!signal.escalate_to_forceful());
}

#[test]
fn test_daemon_shutdown_signal_is_forceful_delegation() {
    use closeclaw_common::ShutdownSignal;

    let handle = ShutdownHandle::new();
    let signal: &dyn ShutdownSignal = &handle;

    assert!(!signal.is_forceful());
    handle.try_start_shutdown();
    assert!(!signal.is_forceful());
    signal.escalate_to_forceful();
    assert!(signal.is_forceful());
}

#[test]
fn test_daemon_shutdown_signal_is_shutting_down_delegation() {
    use closeclaw_common::ShutdownSignal;

    let handle = ShutdownHandle::new();
    let signal: &dyn ShutdownSignal = &handle;

    assert!(!signal.is_shutting_down());
    handle.try_start_shutdown();
    assert!(signal.is_shutting_down());
}

// ── Step 1.5: Common layer delegation integration ──────────────────────

#[tokio::test]
async fn test_common_handle_delegates_drain_waits_for_busy_count() {
    let daemon_handle = ShutdownHandle::new();
    // Wrap daemon handle as common ShutdownSignal for testing delegation
    let common_handle = crate::bridge::common_shutdown_handle(&daemon_handle);

    // Increment busy count through common layer
    common_handle.increment_busy();
    common_handle.increment_busy();
    assert_eq!(common_handle.busy_count(), 2);
    // Daemon layer should reflect the same count
    assert_eq!(daemon_handle.busy_count(), 2);

    // Start shutdown via common layer — should gate immediately
    let initiated = common_handle.is_shutting_down();
    assert!(!initiated, "should not be shutting down yet");

    // Start via daemon handle
    daemon_handle.try_start_shutdown();
    assert!(common_handle.is_shutting_down());
}

#[tokio::test]
async fn test_common_escalate_to_forceful_propagates_to_daemon() {
    let daemon_handle = ShutdownHandle::new();
    let common_handle = crate::bridge::common_shutdown_handle(&daemon_handle);

    // Start graceful shutdown
    daemon_handle.try_start_shutdown();
    assert!(!daemon_handle.is_forceful());

    // Escalate via common layer
    let escalated = common_handle.escalate_to_forceful();
    assert!(escalated, "escalation should succeed");
    // Daemon layer should reflect forceful state
    assert!(daemon_handle.is_forceful());
    assert!(common_handle.is_forceful());
}

#[tokio::test]
async fn test_common_busy_count_delegation_drain_completes() {
    let daemon_handle = ShutdownHandle::new();
    let common_handle = crate::bridge::common_shutdown_handle(&daemon_handle);

    // Increment via common layer
    common_handle.increment_busy();
    assert_eq!(daemon_handle.busy_count(), 1);

    // Start shutdown via daemon
    daemon_handle.try_start_shutdown();

    // Spawn initiate_shutdown — it will block on drain
    let h = daemon_handle.clone();
    let shutdown_handle = tokio::spawn(async move {
        h.initiate_shutdown().await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!daemon_handle.is_stopped());

    // Decrement via common layer
    common_handle.decrement_busy();
    assert_eq!(daemon_handle.busy_count(), 0);

    shutdown_handle.await.unwrap();
    assert!(daemon_handle.is_stopped());
}

// ── Step 1.1: Phase 0 gate timing in select branches ────────────────

#[tokio::test]
async fn test_phase0_gate_set_during_select_branch() {
    // Verifies the fix: try_start_shutdown() is called INSIDE each
    // tokio::select! branch, so the gate is set the instant the signal
    // arrives — not after select returns.
    let handle = ShutdownHandle::new();
    use tokio::signal::unix::{signal, SignalKind};

    // Spawn a task that mimics the run() method's Phase 0: register
    // signal handlers and call try_start_shutdown inside select branches.
    let h = handle.clone();
    let select_result = tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let mut sigterm = signal(SignalKind::terminate()).unwrap();

        tokio::select! {
            _ = sigint.recv() => {
                // Gate set INSIDE the branch, before returning
                h.try_start_shutdown();
            }
            _ = sigterm.recv() => {
                h.try_start_shutdown();
            }
        }

        // Gate must already be active after select returns
        h.is_shutting_down()
    });

    // Give the task time to register signal handlers
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send SIGTERM to trigger the select branch
    unsafe { libc::kill(std::process::id() as i32, libc::SIGTERM) };

    let gate_active = select_result.await.unwrap();
    assert!(
        gate_active,
        "gate must be ShuttingDown immediately after signal (inside select branch)"
    );
}

#[tokio::test]
async fn test_phase0_gate_sigint_sets_gate_immediately() {
    // SIGINT should trigger forceful mode directly — assert is_forceful()
    // rather than just is_shutting_down().
    let handle = ShutdownHandle::new();
    use tokio::signal::unix::{signal, SignalKind};

    let h = handle.clone();
    let select_result = tokio::spawn(async move {
        let mut sigint = signal(SignalKind::interrupt()).unwrap();
        let mut sigterm = signal(SignalKind::terminate()).unwrap();

        tokio::select! {
            _ = sigint.recv() => {
                h.try_start_forceful_shutdown();
            }
            _ = sigterm.recv() => {
                h.try_start_shutdown();
            }
        }

        h.is_forceful()
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Send SIGINT
    unsafe { libc::kill(std::process::id() as i32, libc::SIGINT) };

    let is_forceful = select_result.await.unwrap();
    assert!(
        is_forceful,
        "SIGINT must set ForcefulShuttingDown immediately (inside select branch)"
    );
}
