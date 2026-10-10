//! Agent configuration module
//!
//! Provides AgentsConfigProvider for agents.json (registration list of agent IDs)
//! and AgentDirectoryProvider for loading agent configurations from directories.

mod config_types;
mod directory;
pub(crate) mod jsonc;
mod permission_provider;
mod provider;
mod resolved;
mod types;
mod validation;

// Re-export all config types from the local module.
pub use config_types::*;
pub use directory::AgentDirectoryProvider;
pub(crate) use jsonc::strip_jsonc_comments;
pub use permission_provider::{
    AgentPermissionProvider, LazyAgentPermissions, NoopPermissionProvider,
};
pub use provider::AgentsConfigProvider;
pub use resolved::{from_single, merge};
pub use types::AgentsConfig;
pub use validation::validate_agents_config;

#[cfg(test)]
mod directory_tests;

#[cfg(test)]
mod resolved_tests;

#[cfg(test)]
mod resolved_memory_tests;

#[cfg(test)]
mod permission_provider_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
