//! Test-only [`IdentityResolver`] double used by identity-injection tests.
//!
//! Stands in for the config-backed resolver that the composition root
//! (daemon) builds from `accounts.json` and injects into the plugin.

use std::collections::HashMap;

use closeclaw_common::identity::IdentityResolver;

/// In-memory resolver keyed by the `(platform, bot_app_id, sender_id)`
/// triple, mirroring the config-backed implementation's lookup semantics.
#[derive(Default)]
pub(crate) struct IdentityResolverStub {
    mappings: HashMap<(String, String, String), String>,
}

impl IdentityResolverStub {
    /// Build from `(platform, bot_app_id, sender_id, account_id)` tuples.
    pub(crate) fn new(mappings: &[(&str, &str, &str, &str)]) -> Self {
        Self {
            mappings: mappings
                .iter()
                .map(|&(platform, bot_app_id, sender_id, account_id)| {
                    (
                        (
                            platform.to_string(),
                            bot_app_id.to_string(),
                            sender_id.to_string(),
                        ),
                        account_id.to_string(),
                    )
                })
                .collect(),
        }
    }
}

impl IdentityResolver for IdentityResolverStub {
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
