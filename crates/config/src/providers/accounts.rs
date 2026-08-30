//! Accounts JSON ConfigProvider
//!
//! Loads and validates config/accounts.json configuration.
//! Reuses [`IdentityMapping`] from `identity.rs` as the account entry type.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::identity::IdentityMapping;
use crate::providers::ConfigError;
use crate::ConfigProvider;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Bot-to-Agent binding entry.
///
/// Maps a bot application ID (`bot_app_id`) on an IM platform to a
/// local agent ID. Used by `AccountsConfigProvider` to route incoming
/// messages from a specific bot application to the correct agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BotAgentBinding {
    /// Bot application ID on the IM platform.
    pub bot_app_id: String,
    /// Local agent identifier that the bot is bound to.
    pub agent_id: String,
}

/// Root accounts configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountsConfigData {
    #[serde(default)]
    pub accounts: Vec<IdentityMapping>,

    /// Bot-to-Agent bindings: maps `bot_app_id` to `agent_id`.
    #[serde(default)]
    pub bindings: Vec<BotAgentBinding>,
}

impl AccountsConfigData {
    /// Load from a file path.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path)?;
        Self::from_json_str(&content)
    }

    /// Parse from a JSON string.
    pub fn from_json_str(content: &str) -> Result<Self, ConfigError> {
        let config: AccountsConfigData = serde_json::from_str(content)?;
        Ok(config)
    }

    /// Get a single account by account_id.
    pub fn get_account(&self, account_id: &str) -> Option<&IdentityMapping> {
        self.accounts.iter().find(|a| a.account_id == account_id)
    }

    /// Return all accounts matching the given platform.
    pub fn accounts_by_platform(&self, platform: &str) -> Vec<&IdentityMapping> {
        self.accounts
            .iter()
            .filter(|a| a.platform == platform)
            .collect()
    }

    /// Look up a bot-to-Agent binding by bot application ID.
    pub fn get_binding(&self, bot_app_id: &str) -> Option<&BotAgentBinding> {
        self.bindings.iter().find(|b| b.bot_app_id == bot_app_id)
    }
}

impl ConfigProvider for AccountsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut seen_ids = HashSet::new();
        let mut seen_platform_bindings: HashSet<(String, String, String)> = HashSet::new();

        for (i, account) in self.accounts.iter().enumerate() {
            if account.account_id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: format!("accounts[{}].accountId", i),
                    message: "account_id cannot be empty".to_string(),
                });
            }

            if account.sender_id.is_empty() {
                return Err(ConfigError::ValueError {
                    field: format!("accounts[{}].senderId", i),
                    message: "sender_id cannot be empty".to_string(),
                });
            }

            if !seen_ids.insert(account.account_id.clone()) {
                return Err(ConfigError::ValueError {
                    field: "accountId".to_string(),
                    message: format!(
                        "duplicate account_id '{}' at index {}",
                        account.account_id, i
                    ),
                });
            }

            // Within the same platform, the (bot_app_id, sender_id)
            // combination must be unique.
            let binding_key = (
                account.platform.clone(),
                account.bot_app_id.clone(),
                account.sender_id.clone(),
            );
            if !seen_platform_bindings.insert(binding_key) {
                return Err(ConfigError::ValueError {
                    field: format!("accounts[{}]", i),
                    message: format!(
                        "duplicate binding: platform='{}', \
                         bot_app_id='{}', sender_id='{}' \
                         at index {}",
                        account.platform, account.bot_app_id, account.sender_id, i
                    ),
                });
            }
        }

        Ok(())
    }

    fn config_path() -> &'static str
    where
        Self: Sized,
    {
        "accounts.json"
    }

    fn is_default(&self) -> bool {
        self.accounts.is_empty() && self.bindings.is_empty()
    }
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
