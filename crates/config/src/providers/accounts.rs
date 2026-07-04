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

/// Root accounts configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountsConfigData {
    #[serde(default)]
    pub accounts: Vec<IdentityMapping>,
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
}

impl ConfigProvider for AccountsConfigData {
    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut seen_ids = HashSet::new();

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
        self.accounts.is_empty()
    }
}
