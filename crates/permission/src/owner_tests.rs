//! UT for the Owner identity predicate (`owner::is_owner`).

use crate::owner::is_owner;

#[test]
fn test_literal_owner_id_matches() {
    assert!(is_owner("owner"));
}

#[test]
fn test_other_user_ids_do_not_match() {
    assert!(!is_owner("user-a"));
    assert!(!is_owner("owner2"));
    assert!(!is_owner("owner "));
    assert!(!is_owner(" owner"));
}

#[test]
fn test_empty_and_missing_ids_do_not_match() {
    assert!(!is_owner(""));
}

#[test]
fn test_match_is_case_sensitive() {
    assert!(!is_owner("Owner"));
    assert!(!is_owner("OWNER"));
}
