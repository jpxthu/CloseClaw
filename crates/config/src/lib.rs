//! Configuration management for CloseClaw.
//!
//! Provides config loading, validation, hot-reload, and atomic write
//! for all JSON configuration files under the config/ directory.

pub mod agent_loader;
#[cfg(test)]
mod agent_loader_tests;
pub mod agents;
pub mod backup;
pub mod events;
pub mod identity;
pub mod manager;
pub mod manager_reload;
pub mod migration;
pub mod providers;
pub mod reload_manager;
pub mod session;
pub mod spawn_validation;
pub mod validators;

/// Type alias for a section validator function.
///
/// Used by hot-reload and startup validation.
pub type SectionValidator = dyn Fn(&serde_json::Value) -> Result<(), String>;

// Re-exports from manager
pub use backup::{BackupManager, SafeBackupManager};
pub use events::{ConfigChangeBroadcaster, ConfigChangeEvent};
pub use manager::{
    write_atomically, ConfigInfo, ConfigLoadError, ConfigManager, ConfigSection,
    ConfigValidationError, ConfigWriteError,
};

pub use agents::{AgentDirectoryProvider, AgentsConfig, AgentsConfigProvider};
pub use migration::{migrate_if_needed, ConfigMigrationError};
pub use providers::{
    AccountsConfigData, ChannelsConfigData, ConfigError, ConfigProvider, CredentialsProvider,
    GatewayConfigData, ModelsConfigData, PlanArchiveConfig, RejectionLogConfig, SystemConfigData,
};
pub use reload_manager::{ConfigReloadManager, ReloadCallback, WatcherHandle};
pub use session::{
    IdentifierFormat, JsonSessionConfigProvider, PerAgentSessionConfig, PlanConfig, SessionConfig,
    SessionConfigProvider,
};
pub use spawn_validation::{SpawnError, SpawnValidationResult, SpawnValidator};
