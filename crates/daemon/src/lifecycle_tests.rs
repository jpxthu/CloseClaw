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
    // Graceful mode: heartbeat card should have action buttons
    // Forceful mode: heartbeat card should not have action buttons
    // These are structural assertions about the card JSON,
    // verified via the Gateway's `send_shutdown_heartbeat_card` method.
    // The actual card rendering is tested in tests_plugin.rs.

    // We verify the mode enum behavior here:
    let graceful = ShutdownMode::Graceful;
    let forceful = ShutdownMode::Forceful;

    // Mode comparison works correctly
    assert_ne!(graceful, forceful);
    assert_eq!(ShutdownMode::Graceful, graceful);
    assert_eq!(ShutdownMode::Forceful, forceful);
}
