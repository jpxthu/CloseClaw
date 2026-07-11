//! Agent module - pure configuration layer for agent definitions.

pub mod communication;
pub mod config;
pub mod lookup;
pub mod registry;
pub mod skills_query;
pub mod tools_config_query;

pub use lookup::{AgentConfigInfo, AgentConfigLookup, AgentLookup, AgentRegistryQuery};
pub use skills_query::AgentSkillsQuery;
pub use tools_config_query::{AgentToolsConfig, AgentToolsConfigQuery};
