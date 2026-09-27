//! ConfigProvider implementations

pub mod accounts;
pub mod channels;
pub mod credentials;
pub mod gateway;
pub mod media;
pub mod memory;
pub mod models;
pub mod plugins;
pub mod skills;
pub mod system;

mod error;
mod provider;

pub use accounts::{AccountsConfigData, BotAgentBinding};
pub use channels::ChannelsConfigData;
pub use credentials::CredentialsProvider;
pub use gateway::GatewayConfigData;
pub use media::MediaConfigData;
pub use memory::MemoryConfigData;
pub use models::ModelsConfigData;
pub use plugins::PluginsConfigData;
pub use skills::{SkillsConfig, SkillsConfigData};
pub use system::{AuditLogConfig, PlanArchiveConfig, RejectionLogConfig, SystemConfigData};

pub use error::ConfigError;
pub use provider::ConfigProvider;
