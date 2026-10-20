//! Shared test utilities for permission engine unit tests.

use crate::agent_permissions::DIMENSIONS;

/// Anti-drift guard: `keys` must cover exactly the [`DIMENSIONS`] key set —
/// adding/removing a dimension without updating an explicit key list in a
/// test fixture fails here instead of silently leaving the new dimension
/// untested. Sorted-list equality also fails on duplicate keys.
pub(crate) fn assert_keys_match_dimensions<'a>(keys: impl IntoIterator<Item = &'a str>, ctx: &str) {
    let mut sorted_keys: Vec<&str> = keys.into_iter().collect();
    sorted_keys.sort_unstable();
    let mut dimension_keys = DIMENSIONS.to_vec();
    dimension_keys.sort_unstable();
    assert_eq!(
        sorted_keys, dimension_keys,
        "{ctx} must cover exactly the DIMENSIONS key set"
    );
}
