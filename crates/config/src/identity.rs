//! Config-backed identity resolver for cross-platform user identity mapping.
//!
//! [`IdentityMapping`] and [`ConfigIdentityResolver`] are the configuration
//! representation of identity resolution. The [`IdentityResolver`] trait
//! definition lives in `closeclaw_common::identity`.

use std::collections::HashMap;

use closeclaw_common::identity::IdentityResolver;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// IdentityMapping
// ---------------------------------------------------------------------------

/// A single identity mapping entry loaded from configuration.
///
/// Maps a `(platform, sender_id)` pair to a local `account_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityMapping {
    /// Platform identifier, e.g. `"feishu"`, `"discord"`.
    pub platform: String,

    /// Sender's platform-specific user ID.
    pub sender_id: String,

    /// Local account identifier that the sender maps to.
    pub account_id: String,
}

// ---------------------------------------------------------------------------
// ConfigIdentityResolver
// ---------------------------------------------------------------------------

/// An [`IdentityResolver`] backed by a set of [`IdentityMapping`] entries
/// loaded from a JSON configuration file.
///
/// Internally stores a `HashMap` keyed by `(platform, sender_id)` for
/// O(1) lookups.
#[derive(Debug, Clone)]
pub struct ConfigIdentityResolver {
    mappings: HashMap<(String, String), String>,
}

impl ConfigIdentityResolver {
    /// Build a resolver from a list of mapping entries.
    pub fn new(mappings: Vec<IdentityMapping>) -> Self {
        let map: HashMap<(String, String), String> = mappings
            .into_iter()
            .map(|m| ((m.platform, m.sender_id), m.account_id))
            .collect();
        Self { mappings: map }
    }

    /// Parse a JSON array of mapping entries and build the resolver.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let entries: Vec<IdentityMapping> = serde_json::from_str(json)?;
        Ok(Self::new(entries))
    }

    /// Return the number of configured mappings.
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Return `true` when no mappings are configured.
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
}

impl IdentityResolver for ConfigIdentityResolver {
    fn resolve(&self, platform: &str, sender_id: &str) -> Option<String> {
        self.mappings
            .get(&(platform.to_string(), sender_id.to_string()))
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mappings() -> Vec<IdentityMapping> {
        vec![
            IdentityMapping {
                platform: "feishu".to_string(),
                sender_id: "ou_aaa".to_string(),
                account_id: "local_user_1".to_string(),
            },
            IdentityMapping {
                platform: "feishu".to_string(),
                sender_id: "ou_bbb".to_string(),
                account_id: "local_user_2".to_string(),
            },
            IdentityMapping {
                platform: "discord".to_string(),
                sender_id: "12345".to_string(),
                account_id: "local_user_1".to_string(),
            },
        ]
    }

    #[test]
    fn test_resolve_match() {
        let resolver = ConfigIdentityResolver::new(sample_mappings());
        assert_eq!(
            resolver.resolve("feishu", "ou_aaa"),
            Some("local_user_1".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "12345"),
            Some("local_user_1".to_string())
        );
    }

    #[test]
    fn test_resolve_no_match() {
        let resolver = ConfigIdentityResolver::new(sample_mappings());
        assert_eq!(resolver.resolve("feishu", "ou_unknown"), None);
        assert_eq!(resolver.resolve("slack", "ou_aaa"), None);
    }

    #[test]
    fn test_empty_config() {
        let resolver = ConfigIdentityResolver::new(vec![]);
        assert!(resolver.is_empty());
        assert_eq!(resolver.len(), 0);
        assert_eq!(resolver.resolve("feishu", "ou_aaa"), None);
    }

    #[test]
    fn test_many_to_one() {
        let mappings = vec![
            IdentityMapping {
                platform: "feishu".to_string(),
                sender_id: "ou_aaa".to_string(),
                account_id: "alice".to_string(),
            },
            IdentityMapping {
                platform: "discord".to_string(),
                sender_id: "12345".to_string(),
                account_id: "alice".to_string(),
            },
        ];
        let resolver = ConfigIdentityResolver::new(mappings);
        assert_eq!(
            resolver.resolve("feishu", "ou_aaa"),
            Some("alice".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "12345"),
            Some("alice".to_string())
        );
    }

    #[test]
    fn test_from_json() {
        let json = r#"[
            {"platform":"feishu","sender_id":"ou_xxx","account_id":"local_user_1"},
            {"platform":"discord","sender_id":"42","account_id":"local_user_2"}
        ]"#;
        let resolver = ConfigIdentityResolver::from_json(json).unwrap();
        assert_eq!(resolver.len(), 2);
        assert_eq!(
            resolver.resolve("feishu", "ou_xxx"),
            Some("local_user_1".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "42"),
            Some("local_user_2".to_string())
        );
    }

    #[test]
    fn test_from_json_invalid() {
        let json = r#"not valid json"#;
        assert!(ConfigIdentityResolver::from_json(json).is_err());
    }
}
