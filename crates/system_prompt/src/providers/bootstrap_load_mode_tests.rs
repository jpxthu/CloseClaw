//! Unit tests for `bootstrap_load_mode::resolve_mode`.
//!
//! Truth table over `no_bootstrap` × `session_role` × `bootstrap_mode`.
//! Split from `bootstrap_load_mode.rs` (STANDARDS.md §2: new files must
//! not inline `#[cfg(test)]`); mounted via `#[path]` (bootstrap.rs
//! precedent).

use super::*;

/// no_bootstrap=true → None for every role × bootstrap_mode
/// combination — `bootstrap_mode` must have no effect.
#[test]
fn test_no_bootstrap_true_yields_none_for_all_role_mode_combinations() {
    let combinations = [
        (SessionRole::Main, BootstrapMode::Full),
        (SessionRole::Main, BootstrapMode::Minimal),
        (SessionRole::Sub, BootstrapMode::Full),
        (SessionRole::Sub, BootstrapMode::Minimal),
    ];
    for (role, mode) in combinations {
        assert_eq!(
            resolve_mode(role, mode, true),
            LoadMode::None,
            "no_bootstrap=true must yield None for role={role:?} mode={mode:?}"
        );
    }
}

/// no_bootstrap=false → semantics identical to the pre-existing
/// role guard + configured mode behavior, combination by combination.
#[test]
fn test_no_bootstrap_false_preserves_existing_semantics() {
    // Main follows the configured bootstrap_mode.
    assert_eq!(
        resolve_mode(SessionRole::Main, BootstrapMode::Full, false),
        LoadMode::Full
    );
    assert_eq!(
        resolve_mode(SessionRole::Main, BootstrapMode::Minimal, false),
        LoadMode::Minimal
    );
    // Sub is always forced to Minimal regardless of bootstrap_mode.
    assert_eq!(
        resolve_mode(SessionRole::Sub, BootstrapMode::Full, false),
        LoadMode::Minimal
    );
    assert_eq!(
        resolve_mode(SessionRole::Sub, BootstrapMode::Minimal, false),
        LoadMode::Minimal
    );
}
