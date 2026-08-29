//! Identity mapping and resolution for cross-platform user identity.
//!
//! Provides [`IdentityMapping`] for configuration-driven platform→account_id
//! mapping, [`IdentityResolver`] trait for uniform resolution, and
//! [`ConfigIdentityResolver`] as the default config-backed implementation.

// ---------------------------------------------------------------------------
// IdentityResolver trait
// ---------------------------------------------------------------------------

/// Resolves a `(platform, bot_app_id, sender_id)` triple to a local
/// `account_id`.
///
/// The mapping key is a **(platform, bot_app_id, sender_id) triple**.
/// IM platforms assign distinct sender identifiers per receiving bot
/// application (`bot_app_id`), so the same human user may map to
/// different local accounts depending on which application received the
/// message. Cross-application IDs are not interchangeable.
///
/// Implementations are expected to be constructed at startup with
/// configuration data and remain read-only at runtime.
pub trait IdentityResolver: Send + Sync {
    /// Look up the local `account_id` for the given platform, bot
    /// application, and sender.
    ///
    /// `bot_app_id` identifies the receiving bot application on the IM
    /// platform. An empty string represents the legacy "no application
    /// isolation" configuration.
    ///
    /// Returns `None` when no mapping exists for the triple.
    fn resolve(&self, platform: &str, bot_app_id: &str, sender_id: &str) -> Option<String>;
}
