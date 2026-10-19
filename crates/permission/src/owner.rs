//! Owner identity predicate — the reserved literal `"owner"` marker id.
//!
//! History: the permission engine's Owner shortcut, the gateway slash
//! permission routing short-circuit, and the gateway approval command
//! check used to compare user ids against the literal `"owner"` inline.
//! Those three inline call sites have been migrated to the session-bound
//! criterion — equality with the checkpoint `sender_id`
//! (`SessionManager::get_sender_id`), injected into the engine through
//! the request's `owner_id` field. The migration scope ends there: the
//! literal id no longer grants any privilege **at those three sites**,
//! but it is not a globally unprivileged id — the production sites below
//! still judge literally, and a caller literally named `"owner"` keeps
//! full privilege there:
//!
//! - `crates/permission/src/whitelist.rs` (`caller_to_subject`,
//!   `WhitelistTarget::Auto`): literal `"owner"` caller → `AgentOnly`
//!   whitelist subject.
//! - `crates/gateway/src/session_manager/spawn_adapter.rs`
//!   (`evaluate_user_permissions`): literal `"owner"` skips the User
//!   dimension for spawn permission evaluation.
//! - `crates/gateway/src/slash_permission_handlers.rs` (system command
//!   gate): literal `"owner"` sender passes the gate.
//! - `crates/gateway/src/confirm.rs` (plan confirm/cancel gate):
//!   literal `"owner"` sender passes the gate.
//! - `crates/gateway/src/outbound_helpers.rs` (outbound notification
//!   gating, via this predicate).
//!
//! New code must call [`is_owner`] instead of re-deriving the comparison
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
