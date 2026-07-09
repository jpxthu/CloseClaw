//! Unit tests for daemon lifecycle module

use super::*;
use closeclaw_permission::{Defaults, Effect};
use tempfile::TempDir;

/// Verify `Defaults::user_defaults()` returns all Deny for every field.
/// This is the semantic contract: non-Owner users have no privileges
/// unless explicitly granted.
#[test]
fn test_user_defaults_all_deny() {
    let ud = Defaults::user_defaults();
    assert_eq!(ud.file, Effect::Deny, "user_defaults.file should be Deny");
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
    assert_eq!(engine_default.file, user_default.file);
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

    assert_eq!(ud.file, Effect::Deny);
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
