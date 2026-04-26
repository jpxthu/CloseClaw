//! ConfigProvider implementations

pub mod channels;
pub mod credentials;
pub mod gateway;
pub mod models;
pub mod plugins;
pub mod system;
pub use channels::ChannelsConfigData;
pub use credentials::CredentialsProvider;
pub use gateway::GatewayConfigData;
pub use models::ModelsConfigData;
pub use plugins::PluginsConfigData;
pub use system::SystemConfigData;

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

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Schema validation failed: {0}")]
    SchemaError(String),

    #[error("Invalid value for field '{field}': {message}")]
    ValueError { field: String, message: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),
}
