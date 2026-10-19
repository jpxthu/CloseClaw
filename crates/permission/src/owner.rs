//! Owner identity predicate — the reserved literal `"owner"` marker id.
//!
//! History: the permission engine's Owner shortcut, the gateway slash
//! permission routing short-circuit, and the gateway approval command
//! check used to compare user ids against the literal `"owner"` inline.
//! All three inline call sites have been migrated to the session-bound
//! criterion — equality with the checkpoint `sender_id`
//! (`SessionManager::get_sender_id`), injected into the engine through
//! the request's `owner_id` field — so the literal id no longer grants
//! any privilege by itself.
//!
//! Remaining production caller: `crates/gateway/src/outbound_helpers.rs`
//! (outbound notification gating, kept on the literal criterion). New
//! code must call [`is_owner`] instead of re-deriving the comparison
//! inline.

/// Returns `true` when `user_id` is the reserved literal marker id
/// `"owner"`.
///
/// The production Owner criterion is session-bound (checkpoint
/// `sender_id`, see the module docs); this predicate only reports the
/// literal marker match. The comparison is exact and case-sensitive:
/// `None`/empty/unknown ids (and any other casing) do not match.
pub fn is_owner(user_id: &str) -> bool {
    user_id == "owner"
}
