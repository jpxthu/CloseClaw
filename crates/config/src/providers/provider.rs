//! ConfigProvider trait definition

use crate::providers::ConfigError;

/// Configuration provider trait for extensible config management
pub trait ConfigProvider {
    /// Get config version as string (semver format)
    fn version(&self) -> &'static str;

    /// Validate config schema and values
    fn validate(&self) -> Result<(), ConfigError>;

    /// Get config file path
    fn config_path() -> &'static str
    where
        Self: Sized;

    /// Check if this is the default config
    fn is_default(&self) -> bool;
}
