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
/// Maps a `(platform, bot_app_id, sender_id)` triple to a local
/// `account_id`. An empty `bot_app_id` represents the legacy "no
/// application isolation" configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityMapping {
    /// Platform identifier, e.g. `"feishu"`, `"discord"`.
    pub platform: String,

    /// Receiving bot application ID on the IM platform.
    ///
    /// Defaults to empty string when absent in configuration (serde).
    #[serde(default)]
    pub bot_app_id: String,

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
/// Internally stores a `HashMap` keyed by `(platform, bot_app_id,
/// sender_id)` for O(1) lookups.
#[derive(Debug, Clone)]
pub struct ConfigIdentityResolver {
    mappings: HashMap<(String, String, String), String>,
}

impl ConfigIdentityResolver {
    /// Build a resolver from a list of mapping entries.
    pub fn new(mappings: Vec<IdentityMapping>) -> Self {
        let map: HashMap<(String, String, String), String> = mappings
            .into_iter()
            .map(|m| ((m.platform, m.bot_app_id, m.sender_id), m.account_id))
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
    fn resolve(&self, platform: &str, bot_app_id: &str, sender_id: &str) -> Option<String> {
        self.mappings
            .get(&(
                platform.to_string(),
                bot_app_id.to_string(),
                sender_id.to_string(),
            ))
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
                bot_app_id: String::new(),
                sender_id: "ou_aaa".to_string(),
                account_id: "local_user_1".to_string(),
            },
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: String::new(),
                sender_id: "ou_bbb".to_string(),
                account_id: "local_user_2".to_string(),
            },
            IdentityMapping {
                platform: "discord".to_string(),
                bot_app_id: String::new(),
                sender_id: "12345".to_string(),
                account_id: "local_user_1".to_string(),
            },
        ]
    }

    #[test]
    fn test_resolve_match() {
        let resolver = ConfigIdentityResolver::new(sample_mappings());
        assert_eq!(
            resolver.resolve("feishu", "", "ou_aaa"),
            Some("local_user_1".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "", "12345"),
            Some("local_user_1".to_string())
        );
    }

    #[test]
    fn test_resolve_no_match() {
        let resolver = ConfigIdentityResolver::new(sample_mappings());
        assert_eq!(resolver.resolve("feishu", "", "ou_unknown"), None);
        assert_eq!(resolver.resolve("slack", "", "ou_aaa"), None);
    }

    #[test]
    fn test_empty_config() {
        let resolver = ConfigIdentityResolver::new(vec![]);
        assert!(resolver.is_empty());
        assert_eq!(resolver.len(), 0);
        assert_eq!(resolver.resolve("feishu", "", "ou_aaa"), None);
    }

    #[test]
    fn test_many_to_one() {
        let mappings = vec![
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: String::new(),
                sender_id: "ou_aaa".to_string(),
                account_id: "alice".to_string(),
            },
            IdentityMapping {
                platform: "discord".to_string(),
                bot_app_id: String::new(),
                sender_id: "12345".to_string(),
                account_id: "alice".to_string(),
            },
        ];
        let resolver = ConfigIdentityResolver::new(mappings);
        assert_eq!(
            resolver.resolve("feishu", "", "ou_aaa"),
            Some("alice".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "", "12345"),
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
            resolver.resolve("feishu", "", "ou_xxx"),
            Some("local_user_1".to_string())
        );
        assert_eq!(
            resolver.resolve("discord", "", "42"),
            Some("local_user_2".to_string())
        );
    }

    #[test]
    fn test_from_json_invalid() {
        let json = r#"not valid json"#;
        assert!(ConfigIdentityResolver::from_json(json).is_err());
    }

    // ------------------------------------------------------------------
    // Isolation semantics: same platform+sender_id, different bot_app_id
    // ------------------------------------------------------------------

    /// Core isolation behavior: the same (platform, sender_id) pair maps
    /// to different local accounts depending on which bot_app_id received
    /// the message. This is the fundamental guarantee of the triple key.
    #[test]
    fn test_isolation_different_bot_app_id_yields_different_account() {
        let mappings = vec![
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: "app_x".to_string(),
                sender_id: "ou_alice".to_string(),
                account_id: "alice_via_app_x".to_string(),
            },
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: "app_y".to_string(),
                sender_id: "ou_alice".to_string(),
                account_id: "alice_via_app_y".to_string(),
            },
        ];
        let resolver = ConfigIdentityResolver::new(mappings);

        // Same sender, different bot_app_id → different account_id.
        assert_eq!(
            resolver.resolve("feishu", "app_x", "ou_alice"),
            Some("alice_via_app_x".to_string())
        );
        assert_eq!(
            resolver.resolve("feishu", "app_y", "ou_alice"),
            Some("alice_via_app_y".to_string())
        );

        // Cross-check: the two accounts are distinct.
        assert_ne!(
            resolver.resolve("feishu", "app_x", "ou_alice"),
            resolver.resolve("feishu", "app_y", "ou_alice")
        );
    }

    /// Legacy config compatibility: empty bot_app_id behaves as a separate
    /// key from any non-empty bot_app_id. A user with empty-bot mapping
    /// should NOT collide with the same sender under a real app.
    #[test]
    fn test_isolation_empty_bot_app_id_vs_real_app() {
        let mappings = vec![
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: String::new(), // legacy no-app config
                sender_id: "ou_bob".to_string(),
                account_id: "bob_legacy".to_string(),
            },
            IdentityMapping {
                platform: "feishu".to_string(),
                bot_app_id: "app_real".to_string(),
                sender_id: "ou_bob".to_string(),
                account_id: "bob_real_app".to_string(),
            },
        ];
        let resolver = ConfigIdentityResolver::new(mappings);

        assert_eq!(
            resolver.resolve("feishu", "", "ou_bob"),
            Some("bob_legacy".to_string())
        );
        assert_eq!(
            resolver.resolve("feishu", "app_real", "ou_bob"),
            Some("bob_real_app".to_string())
        );
    }

    /// from_json round-trip: bot_app_id field is preserved through
    /// serialization and correctly used as a resolution key.
    #[test]
    fn test_from_json_with_bot_app_id() {
        let json = r#"[
            {"platform":"feishu","bot_app_id":"app1","sender_id":"ou_x","account_id":"u1"},
            {"platform":"feishu","bot_app_id":"app2","sender_id":"ou_x","account_id":"u2"}
        ]"#;
        let resolver = ConfigIdentityResolver::from_json(json).unwrap();
        assert_eq!(resolver.len(), 2);
        assert_eq!(
            resolver.resolve("feishu", "app1", "ou_x"),
            Some("u1".to_string())
        );
        assert_eq!(
            resolver.resolve("feishu", "app2", "ou_x"),
            Some("u2".to_string())
        );
        // Cross-check: different apps, same sender → different accounts.
        assert_ne!(
            resolver.resolve("feishu", "app1", "ou_x"),
            resolver.resolve("feishu", "app2", "ou_x")
        );
    }
}
