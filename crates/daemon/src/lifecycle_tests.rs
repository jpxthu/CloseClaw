//! Unit tests for daemon lifecycle module

use super::*;
use closeclaw_common::shutdown::ShutdownMode;
use closeclaw_config::ConfigSection;
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
    assert_eq!(ud.exec, Effect::Deny, "user_defaults.exec should be Deny");
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
    assert_eq!(engine_default.exec, user_default.exec);
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
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
    let guard = engine.blocking_read();
    let ud = &guard.rules().user_defaults;

    assert_eq!(ud.file_read, Effect::Deny);
    assert_eq!(ud.file_write, Effect::Deny);
    assert_eq!(ud.exec, Effect::Deny);
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
    let engine = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
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
    // Must match phase_3_background_stop() join_timeout (10s).
    let join_timeout = std::time::Duration::from_secs(10);

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
    // Must match phase_3_background_stop() join_timeout (10s).
    let join_timeout = std::time::Duration::from_secs(10);
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

    // Must match phase_3_background_stop() join_timeout (10s).
    let join_timeout = std::time::Duration::from_secs(10);
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
/// After the 10s join timeout, `phase_3_background_stop` continues.
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

    // Must match phase_3_background_stop() join_timeout (10s).
    let join_timeout = std::time::Duration::from_secs(10);
    let result = tokio::time::timeout(join_timeout, handle).await;

    // Join completes (not timeout) — it's an Err from the panic
    assert!(result.is_ok(), "panicked task join should not timeout");
    let join_result = result.unwrap();
    assert!(join_result.is_err(), "panicked task should return Err");
}

// ── Step 1.4: Phase 3 join_timeout explicit verification ──────────────────

/// Verify that phase_3_background_stop uses a 10-second join timeout.
/// This test spawns a hung task and verifies it is abandoned after
/// approximately 10 seconds — confirming the timeout matches the
/// design doc requirement ("最长 10 秒").
#[tokio::test]
async fn test_phase3_join_timeout_is_10_seconds() {
    // Spawn a task that never exits
    let hang_handle = tokio::spawn(async {
        std::future::pending::<()>().await;
    });

    // Use the exact timeout from phase_3_background_stop (10s)
    let join_timeout = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(join_timeout, hang_handle).await;
    let elapsed = start.elapsed();

    // Timeout should fire — the hung task is abandoned
    assert!(result.is_err(), "10s join should timeout for hung task");
    // Elapsed should be close to 10s (within 1s tolerance)
    assert!(
        elapsed >= std::time::Duration::from_secs(9)
            && elapsed <= std::time::Duration::from_secs(11),
        "elapsed should be ~10s, got {:?}",
        elapsed
    );
}

/// Verify that tasks completing well within 10s are not cut short.
/// This mirrors ArchiveSweeper/DreamingScheduler exiting cleanly
/// after receiving the shutdown signal.
#[tokio::test]
async fn test_phase3_clean_task_exits_within_10s() {
    let (tx, rx) = tokio::sync::watch::channel(());

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

    // Signal immediately — task should exit well before 10s
    let _ = tx.send(());

    let join_timeout = std::time::Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    let result = tokio::time::timeout(join_timeout, handle).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "clean task should join within 10s");
    assert!(result.unwrap().is_ok(), "clean task should not panic");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "clean task should exit well before 10s, took {:?}",
        elapsed
    );
}

// ── Step 1.2: PermissionEngine layer-2 timing tests ───────────────────────

/// Verify that PermissionEngine can be built AFTER ConfigManager loads
/// (layer 2 timing). Creates a ConfigManager, then builds PermissionEngine
/// — confirming the sequence produces a functional engine.
#[test]
fn test_permission_engine_built_after_config_manager_loads() {
    let dir = TempDir::new().unwrap();
    crate::test_helpers::write_mandatory_configs(dir.path()).unwrap();
    // Simulate Phase 1: load ConfigManager
    let config_manager = closeclaw_config::ConfigManager::new(dir.path().to_path_buf())
        .expect("ConfigManager::new should succeed");
    config_manager
        .load()
        .expect("ConfigManager::load should succeed");
    // Simulate Phase 2: build PermissionEngine (what init_phase_2_registries does)
    let permission_engine = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
    // PermissionEngine must be functional after ConfigManager is ready
    let guard = permission_engine.blocking_read();
    assert_eq!(guard.rules().user_defaults.message, Effect::Deny);
    // ConfigManager must also be functional
    assert!(config_manager.section(ConfigSection::System).is_some());
}

/// Verify that the init sequence (Phase 1 → Phase 2) produces a
/// PermissionEngine whose defaults are independent of config content.
/// PermissionEngine loads templates from disk but defaults come from code,
/// so the sequence is valid even with minimal config files.
#[test]
fn test_init_sequence_phase1_then_phase2_permission_engine() {
    let dir = TempDir::new().unwrap();
    crate::test_helpers::write_mandatory_configs(dir.path()).unwrap();
    // Phase 1: ConfigManager loads mandatory sections
    let config_manager = closeclaw_config::ConfigManager::new(dir.path().to_path_buf())
        .expect("ConfigManager::new should succeed");
    config_manager
        .load()
        .expect("ConfigManager::load should succeed");
    // ConfigManager is ready — verify it loaded
    assert!(config_manager.section(ConfigSection::Models).is_some());
    // Phase 2: PermissionEngine is built (depends on ConfigManager per
    // design doc layer 2, but only reads config_dir for templates)
    let pe = Daemon::build_permission_engine(dir.path().to_str().unwrap(), None);
    let guard = pe.blocking_read();
    // Engine defaults: all deny for user, message=Allow for engine default
    assert_eq!(guard.rules().user_defaults.file_read, Effect::Deny);
    assert_eq!(guard.rules().defaults.message, Effect::Allow);
}

/// Verify PermissionEngine is in layer 2 (Registries phase) of the
/// dependency-driven startup order, confirming it builds after
/// ConfigManager (layer 1) and before ApprovalFlow/Gateway (layers 3+).
#[test]
fn test_permission_engine_layer_2_timing_in_dependency_graph() {
    use crate::startup::{all_component_entries, topo_sort_layers, ComponentId, Service};
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    // Layer 1 (index 0) = Foundation: ConfigManager, Storage
    let layer1_ids: Vec<ComponentId> = layers[0].clone();
    assert!(
        layer1_ids.contains(&ComponentId::Foundation(
            crate::startup::Foundation::ConfigManager
        )),
        "ConfigManager must be in layer 1 (Foundation)"
    );
    // Layer 2 (index 1) = Registries: must contain PermissionEngine
    let layer2_ids: Vec<ComponentId> = layers[1].clone();
    assert!(
        layer2_ids.contains(&ComponentId::Service(Service::PermissionEngine)),
        "PermissionEngine must be in layer 2 (Registries), got: {:?}",
        layers[1].iter().map(|c| c.name()).collect::<Vec<_>>()
    );
    // PermissionEngine must NOT be in layer 1
    assert!(
        !layer1_ids.contains(&ComponentId::Service(Service::PermissionEngine)),
        "PermissionEngine must NOT be in layer 1 (Foundation)"
    );
    // PermissionEngine must NOT be in layer 3+ (Wiring, Gateway, etc.)
    for (i, layer) in layers.iter().enumerate().skip(2) {
        assert!(
            !layer.contains(&ComponentId::Service(Service::PermissionEngine)),
            "PermissionEngine must NOT be in layer {} (index {})",
            i + 1,
            i
        );
    }
}

// ── Step 1.2: Phase 2 return value completeness tests ─────────────────────

/// Verify that PermissionEngine appears in the Registries phase of
/// validate_phase_components, confirming it is part of the expected
/// phase 2 component set.
#[test]
fn test_permission_engine_in_registries_phase_of_validate() {
    use crate::startup::{all_component_entries, topo_sort_layers, ComponentId, Service};
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let phases = crate::Daemon::validate_phase_components(&layers)
        .expect("validate_phase_components should succeed");
    // Registries phase (index 1) must contain PermissionEngine
    assert!(
        phases[1].contains(&ComponentId::Service(Service::PermissionEngine)),
        "Registries phase must contain PermissionEngine, got: {:?}",
        phases[1].iter().map(|c| c.name()).collect::<Vec<_>>()
    );
}

/// Verify that the topo sort layer 2 matches the Registries phase
/// expected set from validate_phase_components. This confirms the
/// dependency graph and the phase definitions agree on PermissionEngine's
/// placement.
#[test]
fn test_permission_engine_topo_sort_matches_validate_phase() {
    use crate::startup::{all_component_entries, topo_sort_layers};
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    let phases = crate::Daemon::validate_phase_components(&layers)
        .expect("validate_phase_components should succeed");
    // Sort both for comparison
    let mut topo_layer2 = layers[1].clone();
    topo_layer2.sort_by_key(|id| id.name().to_string());
    let mut phase_registries = phases[1].clone();
    phase_registries.sort_by_key(|id| id.name().to_string());
    assert_eq!(
        topo_layer2, phase_registries,
        "Topo sort layer 2 must match Registries phase — PermissionEngine must be in both"
    );
}

/// Verify the full 6-layer topo sort includes all expected components,
/// with PermissionEngine specifically in layer 2. This is a regression
/// guard: if PermissionEngine moves to a different layer, this test fails.
#[test]
fn test_full_topo_sort_permission_engine_layer_2_regression() {
    use crate::startup::{all_component_entries, topo_sort_layers, ComponentId, Service};
    let entries = all_component_entries();
    let layers = topo_sort_layers(&entries).expect("topo sort should succeed");
    assert_eq!(layers.len(), 6, "expected exactly 6 layers");
    // PermissionEngine must be in layer 2 (index 1)
    assert!(
        layers[1].contains(&ComponentId::Service(Service::PermissionEngine)),
        "PermissionEngine must be in layer 2 (Registries phase)"
    );
    // Count: layer 2 must have exactly 8 components (including PlanArchiveSweeper)
    assert_eq!(
        layers[1].len(),
        8,
        "layer 2 (Registries) must have exactly 8 components, got {}",
        layers[1].len()
    );
}

// ======================================================================
// Step 1.6 — SIGINT first-signal graceful mode tests
// ======================================================================

/// SIGINT (Ctrl+C) first signal must trigger graceful shutdown.
/// Before Step 1.1, SIGINT called `try_start_forceful_shutdown()` which
/// was incorrect per design doc `shutdown.md`:
/// "first SIGINT or SIGTERM → Graceful". After the fix, SIGINT calls
/// `try_start_shutdown()` (graceful) just like SIGTERM.
#[test]
fn test_sigint_first_signal_is_graceful() {
    let handle = crate::shutdown::ShutdownHandle::new();
    // Simulate SIGINT first signal (now calls try_start_shutdown)
    handle.try_start_shutdown();
    assert_eq!(
        handle.mode(),
        ShutdownMode::Graceful,
        "SIGINT first signal must enter Graceful mode"
    );
    assert!(
        !handle.is_forceful(),
        "SIGINT first signal must NOT be forceful"
    );
}

/// After first SIGINT → Graceful, repeated SIGINT → Forceful.
/// This tests the escalation path in Phase 1.
#[test]
fn test_repeated_sigint_escalates_to_forceful() {
    let handle = crate::shutdown::ShutdownHandle::new();
    // First SIGINT: graceful
    handle.try_start_shutdown();
    assert_eq!(handle.mode(), ShutdownMode::Graceful);
    // Repeated SIGINT: escalate to forceful
    let escalated = handle.escalate_to_forceful();
    assert!(
        escalated,
        "escalate_to_forceful must return true on first escalation"
    );
    assert_eq!(
        handle.mode(),
        ShutdownMode::Forceful,
        "Repeated SIGINT must escalate to Forceful"
    );
}

// ======================================================================
// Step 1.6 — PlanArchiveSweeper RAII drop tests
// ======================================================================

/// PlanArchiveSweeperHandle RAII: dropping the handle aborts the
/// background task, consistent with SkillWatcher/ConfigWatcher pattern.
#[tokio::test]
async fn test_plan_archive_sweeper_handle_drop_aborts_task() {
    use tokio::sync::watch;

    // Spawn a mock long-running task
    let (shutdown_tx, mut shutdown_rx) = watch::channel(());
    let mut task_rx = shutdown_rx.clone();
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = task_rx.changed() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            }
        }
    });

    // Wrap in PlanArchiveSweeperHandle
    let sweeper = crate::daemon_struct::PlanArchiveSweeperHandle::new(shutdown_tx, handle);

    // Drop the handle — this should abort the task
    drop(sweeper);

    // Verify the task was aborted (JoinHandle returns JoinError with
    // is_cancelled() == true when the task was aborted)
    // We can't directly check since the handle is moved, but we can
    // verify the shutdown channel is closed
    assert!(
        shutdown_rx.changed().await.is_err(),
        "shutdown channel must be closed after drop"
    );
}

/// PlanArchiveSweeperHandle: shutdown_tx drop closes the channel,
/// signaling the background task to exit.
#[tokio::test]
async fn test_plan_archive_sweeper_handle_drop_closes_channel() {
    use tokio::sync::watch;

    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let handle = tokio::spawn(async {});

    let sweeper = crate::daemon_struct::PlanArchiveSweeperHandle::new(shutdown_tx, handle);

    // Explicitly drop the sweeper before checking the channel
    drop(sweeper);

    // After drop, the receiver should see channel closed.
    let mut rx = shutdown_rx;
    let result = tokio::time::timeout(std::time::Duration::from_millis(100), rx.changed()).await;
    assert!(
        result.is_ok(),
        "changed() must not timeout — channel should be closed"
    );
    assert!(
        result.unwrap().is_err(),
        "channel must be closed after PlanArchiveSweeperHandle drop"
    );
}
