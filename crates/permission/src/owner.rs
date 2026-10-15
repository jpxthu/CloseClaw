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

#[cfg(test)]
mod tests {
    use super::is_owner;

    #[test]
    fn literal_owner_id_matches() {
        assert!(is_owner("owner"));
    }

    #[test]
    fn other_user_ids_do_not_match() {
        assert!(!is_owner("user-a"));
        assert!(!is_owner("owner2"));
        assert!(!is_owner("owner "));
        assert!(!is_owner(" owner"));
    }

    #[test]
    fn empty_and_missing_ids_do_not_match() {
        assert!(!is_owner(""));
    }

    #[test]
    fn match_is_case_sensitive() {
        assert!(!is_owner("Owner"));
        assert!(!is_owner("OWNER"));
    }
}
