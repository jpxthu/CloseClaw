//! Bootstrap load-mode resolution.
//!
//! Pure decision logic for how much of the bootstrap file set a session
//! loads, extracted from [`BootstrapFragmentProvider`](super::bootstrap::BootstrapFragmentProvider)
//! so the truth table (`no_bootstrap` × `session_role` × `bootstrap_mode`)
//! can be unit-tested in isolation (static-layer.md §兜底与变体).
//!
//! Priority order (highest first):
//! 1. `no_bootstrap == true` → the agent declared `noBootstrap: true`; the
//!    session runs as if no bootstrap files existed — load nothing,
//!    regardless of role or `bootstrap_mode` (orthogonal, higher priority).
//! 2. `SessionRole::Sub` → Minimal (role guard; BOOTSTRAP.md only loads
//!    for Main sessions).
//! 3. `SessionRole::Main` → the configured `bootstrap_mode`.

use closeclaw_common::{BootstrapMode, SessionRole};

/// How much of the bootstrap file set a session loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadMode {
    /// Load nothing — the agent declared `noBootstrap: true`;
    /// `bootstrap_mode` has no effect.
    None,
    /// Minimal file set (runtime-required identity/tool files).
    Minimal,
    /// Full file set, including BOOTSTRAP.md.
    Full,
}

/// Resolve the bootstrap load mode for a session.
///
/// * `no_bootstrap == true` → [`LoadMode::None`] — higher priority than
///   `bootstrap_mode`, applies to both roles.
/// * `SessionRole::Sub` → [`LoadMode::Minimal`] regardless of the
///   configured `bootstrap_mode` (existing role-guard semantics).
/// * `SessionRole::Main` → the configured `bootstrap_mode`.
pub fn resolve_mode(
    session_role: SessionRole,
    bootstrap_mode: BootstrapMode,
    no_bootstrap: bool,
) -> LoadMode {
    if no_bootstrap {
        return LoadMode::None;
    }
    match session_role {
        SessionRole::Sub => LoadMode::Minimal,
        SessionRole::Main => match bootstrap_mode {
            BootstrapMode::Minimal => LoadMode::Minimal,
            BootstrapMode::Full => LoadMode::Full,
        },
    }
}

#[cfg(test)]
#[path = "bootstrap_load_mode_tests.rs"]
mod tests;
