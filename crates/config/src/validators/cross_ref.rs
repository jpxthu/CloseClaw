//! Cross-reference data shared between config sections during validation.

use std::collections::HashSet;

/// Cross-reference data for validating binding targets.
pub struct CrossRefData {
    /// Set of registered agent IDs.
    pub agent_ids: HashSet<String>,
    /// Set of registered account IDs.
    pub account_ids: HashSet<String>,
}

/// Set of credential provider names for cross-validation.
pub struct CredentialProviderSet {
    /// Set of credential provider names (map keys from credentials config).
    pub names: HashSet<String>,
}
