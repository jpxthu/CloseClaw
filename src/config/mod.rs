//! Configuration System - hot-reloadable JSON configs
//!
//! Implements ConfigProvider trait for extensible config management.

pub mod agents;
pub mod backup;
pub mod manager;
pub mod migration;
pub mod providers;
pub mod reload;
pub mod session;

// Public re-exports from manager
pub use manager::{
    ConfigInfo, ConfigLoadError, ConfigManager, ConfigSection, ConfigValidationError,
    ConfigWriteError, SafeBackupManager,
};

pub use crate::session::compaction::CompactConfig;
pub use agents::{AgentDirectoryEntry, AgentDirectoryProvider, AgentsConfig, AgentsConfigProvider};
pub use migration::{migrate_if_needed, ConfigMigrationError};
pub use providers::{
    ChannelsConfigData, ConfigError, ConfigProvider, GatewayConfigData, ModelsConfigData,
    SystemConfigData,
};
pub use session::{
    JsonSessionConfigProvider, PerAgentSessionConfig, SessionConfig, SessionConfigProvider,
};

#[cfg(test)]
mod tests {
    use super::*;

    // From tests/smoke_test.rs
    #[test]
    fn test_config_provider_trait_exists() {
        fn _check<T: ConfigProvider>() {}
    }
}
