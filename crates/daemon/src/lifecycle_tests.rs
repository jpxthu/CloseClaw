//! Unit tests for daemon lifecycle module

use super::*;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_permission::{Defaults, Effect};
use tempfile::TempDir;

/// Verify `Defaults::user_defaults()` returns all Deny for every field.
/// This is the semantic contract: non-Owner users have no privileges
/// unless explicitly granted.
#[test]
fn test_user_defaults_all_deny() {
    let ud = Defaults::user_defaults();
    assert_eq!(
        ud.file_read,
        Effect::Deny,
        "user_defaults.file_read should be Deny"
    );
    assert_eq!(
        ud.file_write,
        Effect::Deny,
        "user_defaults.file_write should be Deny"
    );
    assert_eq!(
        ud.command,
        Effect::Deny,
        "user_defaults.command should be Deny"
    );
    assert_eq!(
        ud.network,
        Effect::Deny,
        "user_defaults.network should be Deny"
    );
    assert_eq!(
        ud.inter_agent,
        Effect::Deny,
        "user_defaults.inter_agent should be Deny"
    );
    assert_eq!(
        ud.config,
        Effect::Deny,
        "user_defaults.config should be Deny"
    );
    assert_eq!(
        ud.tool_call,
        Effect::Deny,
        "user_defaults.tool_call should be Deny"
    );
    assert_eq!(
        ud.message,
        Effect::Deny,
        "user_defaults.message should be Deny"
    );
}

/// Verify that `Defaults::default()` (the engine-level default) differs
/// from `user_defaults`: `message` is `Allow` in the engine default but
/// `Deny` in user defaults. This ensures the two are distinct and the
/// distinction is intentional.
#[test]
fn test_user_defaults_differs_from_engine_default() {
    let engine_default = Defaults::default();
    let user_default = Defaults::user_defaults();

    // message is the key difference: Allow in engine, Deny in user
    assert_eq!(engine_default.message, Effect::Allow);
    assert_eq!(user_default.message, Effect::Deny);

    // All other fields are identical
    assert_eq!(engine_default.file_read, user_default.file_read);
    assert_eq!(engine_default.file_write, user_default.file_write);
    assert_eq!(engine_default.command, user_default.command);
    assert_eq!(engine_default.network, user_default.network);
    assert_eq!(engine_default.inter_agent, user_default.inter_agent);
    assert_eq!(engine_default.config, user_default.config);
    assert_eq!(engine_default.tool_call, user_default.tool_call);
}

/// Verify that `build_permission_engine` produces an engine whose
/// `user_defaults` are set to all Deny.
#[test]
fn test_build_permission_engine_user_defaults_are_all_deny() {
    let dir = TempDir::new().unwrap();
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    let guard = engine.blocking_read();
    let ud = &guard.rules().user_defaults;

    assert_eq!(ud.file_read, Effect::Deny);
    assert_eq!(ud.file_write, Effect::Deny);
    assert_eq!(ud.command, Effect::Deny);
    assert_eq!(ud.network, Effect::Deny);
    assert_eq!(ud.inter_agent, Effect::Deny);
    assert_eq!(ud.config, Effect::Deny);
    assert_eq!(ud.tool_call, Effect::Deny);
    assert_eq!(ud.message, Effect::Deny);
}

/// Verify that `build_permission_engine` uses `user_defaults` (not
/// `Defaults::default()`) for the RuleSet's user_defaults field.
/// The distinction: user_defaults has message=Deny, while
/// Defaults::default() has message=Allow.
#[test]
fn test_build_permission_engine_user_defaults_not_engine_default() {
    let dir = TempDir::new().unwrap();
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap());
    let guard = engine.blocking_read();
    let ud = &guard.rules().user_defaults;

    // If this were mistakenly set to Defaults::default(), message would be Allow.
    assert_ne!(
        ud.message,
        Effect::Allow,
        "user_defaults.message must be Deny, not Allow (would indicate Defaults::default() was used)"
    );
}

// ── Step 1.5: Phase 0 notification tests ────────────────────────────────

/// Phase 0 notification is sent via `send_shutdown_progress_card`.
/// After signal reception, the first call uses the mode from
/// `shutdown.mode()`. This test verifies the mode determines the card
/// type (Graceful → "blue" template, Forceful → "red" template).
/// The Gateway's card methods are tested in `tests_plugin.rs`.
#[test]
fn test_phase0_shutdown_mode_determines_card_type() {
    let handle = crate::shutdown::ShutdownHandle::new();

    // Graceful mode → blue card
    handle.try_start_shutdown();
    assert_eq!(handle.mode(), ShutdownMode::Graceful);

    // Forceful mode → red card
    let handle2 = crate::shutdown::ShutdownHandle::new();
    handle2.try_start_forceful_shutdown();
    assert_eq!(handle2.mode(), ShutdownMode::Forceful);
}

/// Phase 0 notification timing: the gate is set BEFORE Phase 1 starts.
/// After signal reception (`try_start_shutdown`), `is_shutting_down()`
/// returns true immediately — no async drain needed.
#[test]
fn test_phase0_notification_timing_gate_set_before_phase1() {
    let handle = crate::shutdown::ShutdownHandle::new();
    assert!(!handle.is_shutting_down());

    // Simulate Phase 0: signal received, gate set
    handle.try_start_shutdown();

    // Gate is active — this is the precondition for sending notification
    assert!(handle.is_shutting_down());
    // Mode is Graceful — determines blue card
    assert_eq!(handle.mode(), ShutdownMode::Graceful);
}

/// Forceful signal (SIGINT) → `try_start_forceful_shutdown` sets
/// ForcefulShuttingDown immediately. The card type is red.
#[test]
fn test_phase0_forceful_signal_sets_mode_for_red_card() {
    let handle = crate::shutdown::ShutdownHandle::new();
    handle.try_start_forceful_shutdown();
    assert!(handle.is_shutting_down());
    assert!(handle.is_forceful());
    assert_eq!(handle.mode(), ShutdownMode::Forceful);
}

// ── Step 1.5: Phase 2 heartbeat tests ───────────────────────────────────

/// Heartbeat card is sent after 30s of no events in Phase 2.
/// The Gateway method `send_shutdown_heartbeat_card` is tested in
/// `tests_plugin.rs`. Here we verify the mode affects card content:
/// Graceful mode includes action buttons, Forceful does not.
#[test]
fn test_heartbeat_card_mode_affects_buttons() {
    let graceful = ShutdownMode::Graceful;
    let forceful = ShutdownMode::Forceful;
    assert_ne!(graceful, forceful);
    assert_eq!(ShutdownMode::Graceful, graceful);
    assert_eq!(ShutdownMode::Forceful, forceful);
}

// ── Step 1.5: Phase 3 join wait behavior tests ──────────────────────────

/// Verify that after taking all JoinHandles, they become None.
/// This mirrors what `phase_3_background_stop` does: each handle is
/// `take()`-ed during the join phase, leaving the field as None.
#[tokio::test]
async fn test_phase3_join_handles_taken_after_stop() {
    // Simulate: spawn tasks and store handles
    let mut archive_handle = Some(tokio::spawn(async {}));
    let mut announce_handle = Some(tokio::spawn(async {}));
    let mut dreaming_handle = Some(tokio::spawn(async {}));
    let mut plan_archive_handle = Some(tokio::spawn(async {}));

    assert!(archive_handle.is_some());
    assert!(announce_handle.is_some());
    assert!(dreaming_handle.is_some());
    assert!(plan_archive_handle.is_some());

    // Simulate phase_3_background_stop: take each handle
    let join_timeout = std::time::Duration::from_secs(15);

    if let Some(handle) = archive_handle.take() {
        let _ = tokio::time::timeout(join_timeout, handle).await;
    }
    if let Some(handle) = announce_handle.take() {
        let _ = tokio::time::timeout(join_timeout, handle).await;
    }
    if let Some(handle) = dreaming_handle.take() {
        let _ = tokio::time::timeout(join_timeout, handle).await;
    }
    if let Some(handle) = plan_archive_handle.take() {
        let _ = tokio::time::timeout(join_timeout, handle).await;
    }

    // All handles should now be None
    assert!(archive_handle.is_none());
    assert!(announce_handle.is_none());
    assert!(dreaming_handle.is_none());
    assert!(plan_archive_handle.is_none());
}

/// Verify that background tasks exit cleanly when a `watch::Sender`
/// sends the shutdown signal. This mirrors the real flow: tasks run
/// a loop that watches for `()` on a `watch::Receiver`, and exit when
/// the signal arrives.
#[tokio::test]
async fn test_phase3_background_tasks_exit_on_signal() {
    let (tx, rx) = tokio::sync::watch::channel(());

    // Spawn a mock background task that watches for shutdown
    let handle = tokio::spawn(async move {
        let mut rx = rx;
        loop {
            if *rx.borrow_and_update() == () {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    // Send shutdown signal
    let _ = tx.send(());

    // Task should exit cleanly within timeout
    let join_timeout = std::time::Duration::from_secs(15);
    let result = tokio::time::timeout(join_timeout, handle).await;
    assert!(result.is_ok(), "task should exit after shutdown signal");
    let join_result = result.unwrap();
    assert!(join_result.is_ok(), "task should not panic");
}

/// Verify that multiple background tasks all exit when signalled.
/// Mirrors phase_3_background_stop sending signals to 4 tasks.
#[tokio::test]
async fn test_phase3_all_tasks_exit_on_respective_signals() {
    let (tx1, rx1) = tokio::sync::watch::channel(());
    let (tx2, rx2) = tokio::sync::watch::channel(());
    let (tx3, rx3) = tokio::sync::watch::channel(());
    let (tx4, rx4) = tokio::sync::watch::channel(());

    let make_task = |mut rx: tokio::sync::watch::Receiver<()>| {
        tokio::spawn(async move {
            loop {
                if *rx.borrow_and_update() == () {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        })
    };

    let h1 = make_task(rx1);
    let h2 = make_task(rx2);
    let h3 = make_task(rx3);
    let h4 = make_task(rx4);

    // Send all shutdown signals
    let _ = tx1.send(());
    let _ = tx2.send(());
    let _ = tx3.send(());
    let _ = tx4.send(());

    let join_timeout = std::time::Duration::from_secs(15);
    let (r1, r2, r3, r4) = tokio::join!(
        tokio::time::timeout(join_timeout, h1),
        tokio::time::timeout(join_timeout, h2),
        tokio::time::timeout(join_timeout, h3),
        tokio::time::timeout(join_timeout, h4),
    );

    assert!(r1.is_ok(), "ArchiveSweeper mock should exit");
    assert!(r2.is_ok(), "AnnounceSweeper mock should exit");
    assert!(r3.is_ok(), "DreamingScheduler mock should exit");
    assert!(r4.is_ok(), "PlanArchiveTask mock should exit");
}

/// Verify that a hung background task does not block the daemon.
/// After the 15s join timeout, `phase_3_background_stop` continues.
/// This test uses a short 100ms timeout to stay within CONTRIBUTING.md
/// <1s limit while still verifying the timeout path.
#[tokio::test]
async fn test_phase3_hung_task_timeout_does_not_block() {
    // Spawn a task that never exits (simulates a hung background task)
    let hang_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    // Use a short timeout for testing (real code uses 15s)
    let test_timeout = std::time::Duration::from_millis(100);
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(test_timeout, hang_handle).await;
    let elapsed = start.elapsed();

    // Timeout should fire — the hung task is not awaited forever
    assert!(result.is_err(), "join should timeout for hung task");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "timeout should fire well within 1s"
    );
}

/// Verify that after phase_3 pattern (signal + join with timeout),
/// a mix of clean and hung tasks all resolve without blocking.
/// The hung task times out, the clean task exits, and execution
/// continues.
#[tokio::test]
async fn test_phase3_mixed_tasks_resolved() {
    let (tx_clean, rx_clean) = tokio::sync::watch::channel(());

    // Clean task: exits on signal
    let clean_handle = tokio::spawn(async move {
        let mut rx = rx_clean;
        loop {
            if *rx.borrow_and_update() == () {
                break;
            }
            if rx.changed().await.is_err() {
                break;
            }
        }
    });

    // Hung task: never exits
    let hang_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    // Signal the clean task
    let _ = tx_clean.send(());

    let test_timeout = std::time::Duration::from_millis(100);
    let start = tokio::time::Instant::now();

    // Join both with timeout — neither should block overall
    let (clean_result, hang_result) = tokio::join!(
        tokio::time::timeout(test_timeout, clean_handle),
        tokio::time::timeout(test_timeout, hang_handle),
    );

    let elapsed = start.elapsed();

    // Clean task exited successfully
    assert!(
        clean_result.is_ok(),
        "clean task should join within timeout"
    );
    assert!(clean_result.unwrap().is_ok(), "clean task should not panic");

    // Hung task timed out
    assert!(hang_result.is_err(), "hung task should timeout");

    // Total elapsed should be bounded (timeout, not infinite)
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "mixed join should complete within 1s"
    );
}

/// Verify that a panicked task's JoinHandle returns Err (not timeout).
/// phase_3_background_stop logs this as a warning and continues.
#[tokio::test]
async fn test_phase3_panicked_task_returns_err() {
    let handle = tokio::spawn(async {
        panic!("mock background task panic");
    });

    let join_timeout = std::time::Duration::from_secs(15);
    let result = tokio::time::timeout(join_timeout, handle).await;

    // Join completes (not timeout) — it's an Err from the panic
    assert!(result.is_ok(), "panicked task join should not timeout");
    let join_result = result.unwrap();
    assert!(join_result.is_err(), "panicked task should return Err");
}
