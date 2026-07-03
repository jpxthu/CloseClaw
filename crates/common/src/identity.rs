//! Identity mapping and resolution for cross-platform user identity.
//!
//! Provides [`IdentityMapping`] for configuration-driven platform→account_id
//! mapping, [`IdentityResolver`] trait for uniform resolution, and
//! [`ConfigIdentityResolver`] as the default config-backed implementation.

// ---------------------------------------------------------------------------
// IdentityResolver trait
// ---------------------------------------------------------------------------

/// Resolves a `(platform, sender_id)` pair to a local `account_id`.
///
/// Implementations are expected to be constructed at startup with
/// configuration data and remain read-only at runtime.
pub trait IdentityResolver: Send + Sync {
    /// Look up the local `account_id` for the given platform and sender.
    ///
    /// Returns `None` when no mapping exists for the pair.
    fn resolve(&self, platform: &str, sender_id: &str) -> Option<String>;
}
