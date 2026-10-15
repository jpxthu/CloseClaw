//! Owner identity predicate — single source of truth.
//!
//! The system-wide Owner criterion is the exact user id `"owner"`,
//! matching the existing inline judgments (permission engine owner
//! short-circuit, slash permission routing, approval flow). New code
//! must call [`is_owner`] instead of re-deriving the comparison
//! inline; refactoring the existing inline call sites is out of scope.

/// Returns `true` when `user_id` denotes the system Owner.
///
/// The comparison is exact and case-sensitive: only the literal id
/// `"owner"` qualifies; `None`/empty/unknown ids (and any other
/// casing) are not the Owner.
pub fn is_owner(user_id: &str) -> bool {
    user_id == "owner"
}
